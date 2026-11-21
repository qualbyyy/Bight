use std::sync::{Mutex, OnceLock};

use vpn_core::ProfileMode;

use crate::{
    core_manager::{self, ConnectOptions},
    state::{ConnectionStatus, SharedAppState},
};

#[cfg(desktop)]
use tauri::{
    App, Emitter, Manager,
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

#[cfg(desktop)]
static STATUS_ITEM: OnceLock<Mutex<Option<MenuItem<tauri::Wry>>>> = OnceLock::new();

#[cfg(desktop)]
pub fn setup(app: &mut App, shared: SharedAppState) -> tauri::Result<()> {
    let handle = app.handle().clone();
    let status = MenuItem::with_id(
        &handle,
        "status",
        status_text(&ConnectionStatus::Disconnected),
        false,
        None::<&str>,
    )?;
    let connect = MenuItem::with_id(&handle, "connect", "Подключить", true, None::<&str>)?;
    let disconnect = MenuItem::with_id(&handle, "disconnect", "Отключить", true, None::<&str>)?;
    let tun = MenuItem::with_id(&handle, "mode-tun", "Режим: TUN", true, None::<&str>)?;
    let proxy = MenuItem::with_id(
        &handle,
        "mode-proxy",
        "Режим: System Proxy",
        true,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(&handle, "open", "Открыть окно", true, None::<&str>)?;
    let quit = MenuItem::with_id(&handle, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(
        &handle,
        &[&status, &connect, &disconnect, &tun, &proxy, &open, &quit],
    )?;

    if let Ok(mut guard) = STATUS_ITEM.get_or_init(|| Mutex::new(None)).lock() {
        guard.replace(status);
    }

    let shared_for_menu = shared.clone();
    TrayIconBuilder::with_id("qonnect-tray")
        .icon(tray_image())
        .tooltip("Qonnect")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| {
            let shared = shared_for_menu.clone();
            let app = app.clone();
            if event.id() == "open" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                return;
            }
            tauri::async_runtime::spawn(async move {
                let result = match () {
                    _ if event.id() == "connect" => {
                        let (node_id, mode) = {
                            let guard = shared.read().await;
                            (guard.profile.active_node_id, guard.profile.mode)
                        };
                        core_manager::connect(
                            &app,
                            &shared,
                            ConnectOptions {
                                node_id,
                                mode: Some(mode),
                            },
                        )
                        .await
                        .map(|_| ())
                    }
                    _ if event.id() == "disconnect" => {
                        core_manager::disconnect(&app, &shared).await
                    }
                    _ if event.id() == "mode-tun" => {
                        core_manager::set_mode(&app, &shared, ProfileMode::Tun).await
                    }
                    _ if event.id() == "mode-proxy" => {
                        core_manager::set_mode(&app, &shared, ProfileMode::SystemProxy).await
                    }
                    _ if event.id() == "quit" => {
                        let result = core_manager::disconnect(&app, &shared).await;
                        app.exit(0);
                        result
                    }
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    let _ = app.emit("backend-error", error.to_string());
                }
            });
        })
        .build(app)?;
    Ok(())
}

pub fn refresh_status(status: &ConnectionStatus) {
    #[cfg(desktop)]
    if let Some(lock) = STATUS_ITEM.get()
        && let Ok(guard) = lock.lock()
        && let Some(item) = guard.as_ref()
    {
        let _ = item.set_text(status_text(status));
    }

    #[cfg(not(desktop))]
    let _ = status;
}

#[cfg(desktop)]
fn status_text(status: &ConnectionStatus) -> String {
    match status {
        ConnectionStatus::Disconnected => "Статус: отключен".to_string(),
        ConnectionStatus::Connecting => "Статус: подключение".to_string(),
        ConnectionStatus::Connected { mode, .. } => {
            format!("Статус: подключен ({})", mode_label(*mode))
        }
        ConnectionStatus::Error { message } => format!("Статус: ошибка — {message}"),
    }
}

#[cfg(desktop)]
fn mode_label(mode: ProfileMode) -> &'static str {
    match mode {
        ProfileMode::Tun => "TUN",
        ProfileMode::SystemProxy => "System Proxy",
    }
}

#[cfg(desktop)]
fn tray_image() -> Image<'static> {
    let size = 16_u32;
    let mut rgba = vec![0_u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dx = x as i32 - 7;
            let dy = y as i32 - 7;
            if dx * dx + dy * dy <= 49 {
                let offset = ((y * size + x) * 4) as usize;
                rgba[offset] = 39;
                rgba[offset + 1] = 174;
                rgba[offset + 2] = 156;
                rgba[offset + 3] = 255;
            }
        }
    }
    Image::new_owned(rgba, size, size)
}
