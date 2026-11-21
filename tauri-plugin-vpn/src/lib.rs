mod error;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

pub use error::{Error, Result};

#[cfg(desktop)]
use desktop::Vpn;
#[cfg(mobile)]
use mobile::Vpn;

use tauri::{Manager, Runtime, plugin::{Builder, TauriPlugin}};

pub trait VpnExt<R: Runtime> {
    fn vpn(&self) -> &Vpn<R>;
}

impl<R: Runtime, T: Manager<R>> VpnExt<R> for T {
    fn vpn(&self) -> &Vpn<R> {
        self.state::<Vpn<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("vpn")
        .setup(|app, api| {
            #[cfg(desktop)]
            let _ = api;
            #[cfg(target_os = "android")]
            let vpn = mobile::init(app, api)?;
            #[cfg(desktop)]
            let vpn = desktop::init::<R>();
            #[cfg(all(mobile, not(target_os = "android")))]
            let vpn = mobile::init(app, api)?;
            app.manage(vpn);
            Ok(())
        })
        .build()
}
