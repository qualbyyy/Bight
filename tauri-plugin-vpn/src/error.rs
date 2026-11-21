use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("VPN plugin is only available on Android")]
    UnsupportedPlatform,
    #[cfg(mobile)]
    #[error(transparent)]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

pub type Result<T> = std::result::Result<T, Error>;
