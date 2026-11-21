use std::marker::PhantomData;

use tauri::Runtime;

use crate::{Error, Result};

pub struct Vpn<R: Runtime>(PhantomData<fn() -> R>);

pub fn init<R: Runtime>() -> Vpn<R> {
    Vpn(PhantomData)
}

impl<R: Runtime> Vpn<R> {
    pub fn start_vpn(&self, _config: String) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }

    pub fn stop_vpn(&self) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }

    pub fn status(&self) -> Result<serde_json::Value> {
        Err(Error::UnsupportedPlatform)
    }
}
