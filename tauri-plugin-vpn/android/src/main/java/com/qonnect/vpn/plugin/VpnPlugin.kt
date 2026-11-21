package com.qonnect.vpn.plugin

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import androidx.activity.result.ActivityResult
import androidx.core.content.ContextCompat
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class StartVpnArgs {
    var config: String = ""
}

@TauriPlugin
class VpnPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun startVpn(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(StartVpnArgs::class.java)
            require(args.config.isNotBlank()) { "VPN config is empty" }
            val configPath = writeActiveConfig(args.config)
            val permissionIntent = VpnService.prepare(activity)
            if (permissionIntent != null) {
                startActivityForResult(invoke, permissionIntent, "vpnPermissionResult")
            } else {
                startService(configPath)
                invoke.resolve(statePayload("starting"))
            }
        } catch (error: Exception) {
            invoke.reject(error.message ?: "Failed to start VPN", error)
        }
    }

    @ActivityCallback
    fun vpnPermissionResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.reject("VPN permission was denied")
            return
        }
        try {
            val configPath = File(activity.filesDir, ACTIVE_CONFIG_FILE).absolutePath
            startService(configPath)
            invoke.resolve(statePayload("starting"))
        } catch (error: Exception) {
            invoke.reject(error.message ?: "Failed to start VPN", error)
        }
    }

    @Command
    fun stopVpn(invoke: Invoke) {
        try {
            activity.startService(
                Intent(activity, CoreVpnService::class.java).setAction(CoreVpnService.ACTION_STOP),
            )
            invoke.resolve(statePayload("stopping"))
        } catch (error: Exception) {
            invoke.reject(error.message ?: "Failed to stop VPN", error)
        }
    }

    @Command
    fun status(invoke: Invoke) {
        invoke.resolve(statePayload(CoreVpnService.state))
    }

    private fun writeActiveConfig(config: String): String {
        val target = File(activity.filesDir, ACTIVE_CONFIG_FILE)
        val temporary = File(activity.filesDir, "$ACTIVE_CONFIG_FILE.tmp")
        temporary.writeText(config)
        check(temporary.renameTo(target)) { "Could not atomically replace active VPN config" }
        return target.absolutePath
    }

    private fun startService(configPath: String) {
        val intent = Intent(activity, CoreVpnService::class.java)
            .putExtra(CoreVpnService.EXTRA_CONFIG_PATH, configPath)
        ContextCompat.startForegroundService(activity, intent)
    }

    private fun statePayload(value: String): JSObject = JSObject().apply {
        put("state", value)
    }

    private companion object {
        const val ACTIVE_CONFIG_FILE = "active_config.json"
    }
}
