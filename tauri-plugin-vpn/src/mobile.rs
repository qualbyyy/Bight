use serde::de::DeserializeOwned;
use serde_json::json;
use tauri::{AppHandle, Runtime, plugin::{PluginApi, PluginHandle}};

use crate::Result;

pub struct Vpn<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<Vpn<R>> {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        return Ok(Vpn(api.register_android_plugin(
            "com.qonnect.vpn.plugin",
            "VpnPlugin",
        )?));
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, api);
        Err(crate::Error::UnsupportedPlatform)
    }
}

impl<R: Runtime> Vpn<R> {
    pub fn start_vpn(&self, config: String) -> Result<()> {
        self.0
            .run_mobile_plugin("startVpn", json!({ "config": config }))
            .map_err(Into::into)
    }

    pub fn stop_vpn(&self) -> Result<()> {
        self.0
            .run_mobile_plugin("stopVpn", json!({}))
            .map_err(Into::into)
    }

    pub fn status(&self) -> Result<serde_json::Value> {
        self.0
            .run_mobile_plugin("status", json!({}))
            .map_err(Into::into)
    }
}
