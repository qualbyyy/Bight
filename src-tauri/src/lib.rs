use tauri::Manager;

pub mod commands;
mod core_manager;
mod state;
mod tray;

pub fn run() {
    #[cfg(mobile)]
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_vpn::init())
        .plugin(tauri_plugin_barcode_scanner::init());
    #[cfg(not(mobile))]
    let builder = tauri::Builder::default().plugin(tauri_plugin_vpn::init());
    builder
        .setup(|app| {
            let shared = state::load_app_state(app.handle())?;
            app.manage(shared.clone());
            tauri::async_runtime::block_on(core_manager::cleanup_stale_system_proxy(app.handle()))?;
            #[cfg(desktop)]
            tray::setup(app, shared)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_profile,
            commands::list_subscriptions,
            commands::import_subscription,
            commands::refresh_subscription,
            commands::delete_subscription,
            commands::import_node_uri,
            commands::list_nodes,
            commands::create_node,
            commands::update_node,
            commands::delete_node,
            commands::list_rules,
            commands::create_rule,
            commands::update_rule,
            commands::delete_rule,
            commands::connect,
            commands::disconnect,
            commands::get_status,
            commands::get_logs,
            commands::set_mode,
            commands::ping_node,
            commands::ping_all,
            commands::get_active_connections,
            commands::sync_start_host,
            commands::sync_stop_host,
            commands::sync_discover_peers,
            commands::sync_connect_peer,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Bight")
        .run(|app_handle, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                let shared = app_handle.state::<state::SharedAppState>().inner().clone();
                let _ =
                    tauri::async_runtime::block_on(core_manager::disconnect(app_handle, &shared));
            }
        });
}
