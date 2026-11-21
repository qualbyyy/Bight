package com.qonnect.vpn.plugin

import android.net.ConnectivityManager
import android.net.IpPrefix
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.Process
import android.os.Handler
import android.os.Looper
import android.system.Os
import android.system.OsConstants
import android.util.Log
import io.nekohasekai.libbox.BridgeOptions
import io.nekohasekai.libbox.BridgeSession
import io.nekohasekai.libbox.CommandServerHandler
import io.nekohasekai.libbox.ConnectionOwner
import io.nekohasekai.libbox.InterfaceUpdateListener
import io.nekohasekai.libbox.Libbox
import io.nekohasekai.libbox.LocalDNSTransport
import io.nekohasekai.libbox.NeighborUpdateListener
import io.nekohasekai.libbox.NetworkInterface as LibboxNetworkInterface
import io.nekohasekai.libbox.NetworkInterfaceIterator
import io.nekohasekai.libbox.Notification
import io.nekohasekai.libbox.PlatformInterface
import io.nekohasekai.libbox.PlatformUser
import io.nekohasekai.libbox.RoutePrefix
import io.nekohasekai.libbox.ShellSession
import io.nekohasekai.libbox.StringIterator
import io.nekohasekai.libbox.SystemProxyStatus
import io.nekohasekai.libbox.TunOptions
import io.nekohasekai.libbox.WIFIState
import java.net.InetAddress
import java.net.InetSocketAddress

class LibboxBridge(private val service: CoreVpnService) : PlatformInterface, CommandServerHandler {
    private val connectivity = service.getSystemService(ConnectivityManager::class.java)
    private var tunFd: ParcelFileDescriptor? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null

    override fun localDNSTransport(): LocalDNSTransport? = null

    override fun usePlatformAutoDetectInterfaceControl(): Boolean = true

    override fun autoDetectInterfaceControl(fd: Int) {
        check(service.protect(fd)) { "android: could not protect sing-box socket" }
    }

    override fun openTun(options: TunOptions): Int {
        check(VpnService.prepare(service) == null) { "android: missing VPN permission" }
        val builder = service.Builder()
            .setSession("Bight")
            .setMtu(options.mtu)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) builder.setMetered(false)

        addAddresses(builder, options)
        addDnsServers(builder, options)
        if (options.autoRoute) addRoutes(builder, options)

        runCatching { builder.addDisallowedApplication(service.packageName) }
        addPackages(builder, options)
        tunFd?.close()
        tunFd = builder.establish() ?: error("android: failed to establish TUN")
        return tunFd!!.fd
    }

    override fun findConnectionOwner(
        ipProtocol: Int,
        sourceAddress: String,
        sourcePort: Int,
        destinationAddress: String,
        destinationPort: Int,
    ): ConnectionOwner {
        val owner = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val uid = connectivity.getConnectionOwnerUid(
                ipProtocol,
                InetSocketAddress(sourceAddress, sourcePort),
                InetSocketAddress(destinationAddress, destinationPort),
            )
            check(uid != Process.INVALID_UID) { "android: connection owner not found" }
            ConnectionOwner().apply { userId = uid }
        } else {
            Libbox.findConnectionOwner(
                ipProtocol,
                sourceAddress,
                sourcePort,
                destinationAddress,
                destinationPort,
            )
        }
        val packages = service.packageManager.getPackagesForUid(owner.userId)?.toList().orEmpty()
        owner.userName = packages.firstOrNull().orEmpty()
        owner.setAndroidPackageNames(StringArray(packages.iterator()))
        return owner
    }

    override fun startDefaultInterfaceMonitor(listener: InterfaceUpdateListener) {
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = report(network, listener)
            override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) =
                report(network, listener)

            override fun onLost(network: Network) {
                listener.updateDefaultInterface("", -1, false, false)
            }
        }
        networkCallback = callback
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            connectivity.registerBestMatchingNetworkCallback(request, callback, Handler(Looper.getMainLooper()))
        } else {
            connectivity.registerNetworkCallback(request, callback)
        }
    }

    override fun closeDefaultInterfaceMonitor(listener: InterfaceUpdateListener) {
        networkCallback?.let { runCatching { connectivity.unregisterNetworkCallback(it) } }
        networkCallback = null
    }

    override fun getInterfaces(): NetworkInterfaceIterator {
        val interfaces = connectivity.allNetworks.mapNotNull { network ->
            val link = connectivity.getLinkProperties(network) ?: return@mapNotNull null
            val capabilities = connectivity.getNetworkCapabilities(network) ?: return@mapNotNull null
            val name = link.interfaceName ?: return@mapNotNull null
            val index = runCatching { Os.if_nametoindex(name) }.getOrDefault(0)
            if (index == 0) return@mapNotNull null
            LibboxNetworkInterface().apply {
                this.name = name
                this.index = index
                this.mtu = if (link.mtu > 0) link.mtu else 1500
                this.addresses = StringArray(
                    link.linkAddresses.map { "${it.address.hostAddress}/${it.prefixLength}" }.iterator(),
                )
                this.dnsServer = StringArray(
                    link.dnsServers.mapNotNull { it.hostAddress }.iterator(),
                )
                this.gateway = StringArray(
                    link.routes.mapNotNull { it.gateway?.hostAddress }.filter { it != "0.0.0.0" }.iterator(),
                )
                this.flags = interfaceFlags(name, capabilities)
                this.metered = !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
                this.type = when {
                    capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> Libbox.InterfaceTypeWIFI
                    capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> Libbox.InterfaceTypeCellular
                    capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> Libbox.InterfaceTypeEthernet
                    else -> Libbox.InterfaceTypeOther
                }
            }
        }
        return object : NetworkInterfaceIterator {
            private val iterator = interfaces.iterator()
            override fun hasNext(): Boolean = iterator.hasNext()
            override fun next(): LibboxNetworkInterface = iterator.next()
        }
    }

    override fun underNetworkExtension(): Boolean = false

    override fun includeAllNetworks(): Boolean = false

    override fun readWIFIState(): WIFIState? = null

    override fun clearDNSCache() = Unit

    override fun sendNotification(notification: Notification?) = Unit

    override fun cancelNotification(identifier: String?, typeID: Int) = Unit

    override fun startNeighborMonitor(listener: NeighborUpdateListener?) = Unit

    override fun closeNeighborMonitor(listener: NeighborUpdateListener?) = Unit

    override fun registerMyInterface(name: String?) = Unit

    override fun usePlatformShell(): Boolean = false

    override fun checkPlatformShell() = Unit

    override fun openShellSession(
        user: PlatformUser?,
        command: String?,
        environ: StringIterator?,
        term: String?,
        rows: Int,
        cols: Int,
    ): ShellSession = error("android: platform shell is not available")

    override fun lookupUser(username: String?): PlatformUser = error("android: user lookup is not available")

    override fun lookupSFTPServer(): String = error("android: SFTP is not available")

    override fun readSystemSSHHostKey(): String = error("android: SSH host key is not available")

    override fun tailscaleHostname(): String = ""

    override fun usePlatformBridge(): Boolean = false

    override fun createBridge(options: BridgeOptions?): BridgeSession =
        error("android: platform bridge is not available")

    override fun serviceStop() {
        service.stopTunnel()
    }

    override fun serviceReload() {
        service.reloadTunnel()
    }

    override fun getSystemProxyStatus(): SystemProxyStatus? = SystemProxyStatus().apply {
        available = false
        enabled = false
    }

    override fun setSystemProxyEnabled(enabled: Boolean) = Unit

    override fun triggerNativeCrash() = Unit

    override fun writeDebugMessage(message: String?) {
        Log.d("BightVpn", message.orEmpty())
    }

    override fun connectSSHAgent(): Int = -1

    fun close() {
        networkCallback?.let { runCatching { connectivity.unregisterNetworkCallback(it) } }
        networkCallback = null
        tunFd?.close()
        tunFd = null
    }

    private fun addAddresses(builder: VpnService.Builder, options: TunOptions) {
        val inet4 = options.inet4Address
        while (inet4.hasNext()) {
            val address = inet4.next()
            builder.addAddress(address.address(), address.prefix())
        }
        val inet6 = options.inet6Address
        while (inet6.hasNext()) {
            val address = inet6.next()
            builder.addAddress(address.address(), address.prefix())
        }
    }

    private fun addDnsServers(builder: VpnService.Builder, options: TunOptions) {
        val dnsServers = options.dnsServerAddress
        while (dnsServers.hasNext()) builder.addDnsServer(dnsServers.next())
    }

    private fun addRoutes(builder: VpnService.Builder, options: TunOptions) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            addRoutePrefixes(builder, options.inet4RouteAddress)
            addRoutePrefixes(builder, options.inet6RouteAddress)
            addExcludedPrefixes(builder, options.inet4RouteExcludeAddress)
            addExcludedPrefixes(builder, options.inet6RouteExcludeAddress)
        } else {
            addRoutePrefixesLegacy(builder, options.inet4RouteRange)
            addRoutePrefixesLegacy(builder, options.inet6RouteRange)
        }
    }

    private fun addRoutePrefixes(builder: VpnService.Builder, iterator: io.nekohasekai.libbox.RoutePrefixIterator) {
        while (iterator.hasNext()) {
            val prefix = iterator.next()
            builder.addRoute(prefix.address(), prefix.prefix())
        }
    }

    private fun addExcludedPrefixes(
        builder: VpnService.Builder,
        iterator: io.nekohasekai.libbox.RoutePrefixIterator,
    ) {
        while (iterator.hasNext()) builder.excludeRoute(iterator.next().toIpPrefix())
    }

    private fun addRoutePrefixesLegacy(
        builder: VpnService.Builder,
        iterator: io.nekohasekai.libbox.RoutePrefixIterator,
    ) {
        while (iterator.hasNext()) {
            val prefix = iterator.next()
            builder.addRoute(prefix.address(), prefix.prefix())
        }
    }

    private fun addPackages(builder: VpnService.Builder, options: TunOptions) {
        val include = options.includePackage
        while (include.hasNext()) runCatching { builder.addAllowedApplication(include.next()) }
        val exclude = options.excludePackage
        while (exclude.hasNext()) runCatching { builder.addDisallowedApplication(exclude.next()) }
    }

    private fun report(network: Network, listener: InterfaceUpdateListener) {
        val link = connectivity.getLinkProperties(network) ?: return
        val name = link.interfaceName ?: return
        val index = runCatching { Os.if_nametoindex(name) }.getOrDefault(0)
        if (index == 0) return
        val capabilities = connectivity.getNetworkCapabilities(network)
        val expensive = capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true
        listener.updateDefaultInterface(name, index, expensive, false)
    }

    private fun interfaceFlags(name: String, capabilities: NetworkCapabilities): Int {
        var flags = 0
        if (capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) {
            flags = flags or OsConstants.IFF_UP or OsConstants.IFF_RUNNING
        }
        if (name == "lo") flags = flags or OsConstants.IFF_LOOPBACK
        if (name.startsWith("tun") || name.startsWith("ppp")) {
            flags = flags or OsConstants.IFF_POINTOPOINT
        } else {
            flags = flags or OsConstants.IFF_MULTICAST
        }
        return flags
    }

    private class StringArray(private val iterator: Iterator<String>) : StringIterator {
        override fun len(): Int = 0
        override fun hasNext(): Boolean = iterator.hasNext()
        override fun next(): String = iterator.next()
    }

    private fun RoutePrefix.toIpPrefix(): IpPrefix =
        IpPrefix(InetAddress.getByName(address()), prefix())
}
