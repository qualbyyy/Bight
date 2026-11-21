package com.qonnect.vpn.plugin

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import io.nekohasekai.libbox.CommandServer
import io.nekohasekai.libbox.Libbox
import io.nekohasekai.libbox.OverrideOptions
import io.nekohasekai.libbox.SetupOptions
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

class CoreVpnService : VpnService() {
    companion object {
        const val ACTION_STOP = "com.qonnect.vpn.plugin.STOP"
        const val EXTRA_CONFIG_PATH = "com.qonnect.vpn.plugin.CONFIG_PATH"
        const val NOTIFICATION_ID = 1701

        @Volatile
        var state: String = "stopped"
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var commandServer: CommandServer? = null
    private var bridge: LibboxBridge? = null
    private var activeConfig: String? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopTunnel()
            return START_NOT_STICKY
        }
        if (state != "stopped") return START_STICKY

        val configPath = intent?.getStringExtra(EXTRA_CONFIG_PATH)
            ?: File(filesDir, "active_config.json").absolutePath
        state = "starting"
        startForegroundCompat(notification("Подключение…"))
        scope.launch { startTunnel(configPath) }
        return START_STICKY
    }

    private suspend fun startTunnel(configPath: String) {
        try {
            val config = normalizeConfig(File(configPath).readText())
            Libbox.setup(
                SetupOptions().apply {
                    basePath = filesDir.absolutePath
                    workingPath = File(filesDir, "sing-box").apply { mkdirs() }.absolutePath
                    tempPath = cacheDir.absolutePath
                    fixAndroidStack = true
                },
            )
            val nextBridge = LibboxBridge(this@CoreVpnService)
            val nextServer = CommandServer(nextBridge, nextBridge)
            nextServer.start()
            nextServer.startOrReloadService(config, OverrideOptions())
            bridge = nextBridge
            commandServer = nextServer
            activeConfig = config
            state = "connected"
            updateNotification("Подключено")
        } catch (error: Throwable) {
            Log.e("BightVpn", "VPN start failed", error)
            state = "error"
            stopTunnel()
        }
    }

    fun reloadTunnel() {
        val server = commandServer ?: return
        val config = activeConfig ?: return
        scope.launch {
            runCatching { server.startOrReloadService(config, OverrideOptions()) }
                .onFailure { Log.e("BightVpn", "VPN reload failed", it) }
        }
    }

    fun stopTunnel() {
        val server = commandServer
        val currentBridge = bridge
        commandServer = null
        bridge = null
        activeConfig = null
        state = "stopped"
        scope.launch {
            runCatching { server?.closeService() }
            runCatching { server?.close() }
            runCatching { currentBridge?.close() }
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    override fun onDestroy() {
        stopTunnel()
        scope.cancel()
        super.onDestroy()
    }

    override fun onRevoke() {
        stopTunnel()
        super.onRevoke()
    }

    override fun onBind(intent: Intent): IBinder? = super.onBind(intent)

    private fun startForegroundCompat(notification: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun notification(text: String): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(
                    Config.NOTIFICATION_CHANNEL,
                    getString(R.string.vpn_notification_channel),
                    NotificationManager.IMPORTANCE_LOW,
                ),
            )
        }
        val openIntent = PendingIntent.getActivity(
            this,
            0,
            packageManager.getLaunchIntentForPackage(packageName),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stopIntent = PendingIntent.getService(
            this,
            1,
            Intent(this, CoreVpnService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, Config.NOTIFICATION_CHANNEL)
            .setSmallIcon(R.drawable.ic_bight_notification)
            .setContentTitle(getString(R.string.vpn_notification_title))
            .setContentText(text)
            .setContentIntent(openIntent)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(0, "Отключить", stopIntent)
            .build()
    }

    private fun updateNotification(text: String) {
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, notification(text))
    }

    private fun normalizeConfig(raw: String): String {
        val root = JSONObject(raw)
        root.optJSONObject("route")?.remove("auto_detect_interface")
        val experimental = root.optJSONObject("experimental") ?: JSONObject().also {
            root.put("experimental", it)
        }
        val cacheFile = experimental.optJSONObject("cache_file") ?: JSONObject().also {
            experimental.put("cache_file", it)
        }
        cacheFile.put("enabled", true)
        cacheFile.put("path", File(filesDir, "cache.db").absolutePath)
        val inbounds = root.optJSONArray("inbounds") ?: return root.toString()
        for (index in 0 until inbounds.length()) {
            val inbound = inbounds.optJSONObject(index) ?: continue
            if (inbound.optString("type") != "tun") continue
            val excludes = inbound.optJSONArray("route_exclude_address") ?: JSONArray().also {
                inbound.put("route_exclude_address", it)
            }
            Config.RFC1918_EXCLUDES.forEach { value ->
                if (!contains(excludes, value)) excludes.put(value)
            }
        }
        return root.toString()
    }

    private fun contains(values: JSONArray, target: String): Boolean {
        for (index in 0 until values.length()) {
            if (values.optString(index) == target) return true
        }
        return false
    }

    private object Config {
        const val NOTIFICATION_CHANNEL = "bight-vpn"
        val RFC1918_EXCLUDES = listOf(
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "169.254.0.0/16",
            "127.0.0.0/8",
            "fc00::/7",
            "fe80::/10",
        )
    }
}
