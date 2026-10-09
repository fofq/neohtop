#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! NeoHtop - A modern system monitor built with Tauri
//!
//! This is the main entry point for the application. It sets up the Tauri
//! application, initializes plugins, and configures window effects.

mod commands;
mod monitoring;
mod state;
mod ui;

use state::AppState;
use tauri::Manager;

/// Main entry point for the application
///
/// # Panics
///
/// Will panic if:
/// - Unable to create the main window
/// - Failed to apply window effects
/// - Failed to initialize the application state
fn main() {
    #[cfg(target_os = "linux")]
    if std::env::var("XDG_SESSION_TYPE").unwrap_or_default() == "wayland" {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            ui::setup_window_effects(&window).expect("Failed to apply window effects");
            // The window is created hidden (visible:false in tauri.conf.json)
            // so the window-state plugin restores the last saved size/position
            // before the first paint; reveal it only once that is done
            window.show().expect("Failed to show the main window");
            let _ = window.set_focus();
            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_os::init())
        // Restores the main window's last size/position while it is still
        // hidden and saves it on exit; the config's 1280×860 is only the
        // first run. VISIBLE is dropped from the flags so the plugin never
        // shows the window itself — the setup hook does it after the
        // acrylic effect is applied.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        .difference(tauri_plugin_window_state::StateFlags::VISIBLE),
                )
                .build(),
        )
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_processes,
            commands::kill_process,
            commands::kill_process_tree,
            commands::kill_app_family,
            commands::deep_kill_process,
            commands::ping_host,
            commands::restart_process,
            commands::get_network_ports,
            commands::get_traffic_counters,
            commands::get_listening_ports,
            commands::identify_port,
            commands::list_container_ports,
            commands::suspend_process,
            commands::resume_process,
            commands::get_process_priority_info,
            commands::set_process_priority,
            commands::set_process_affinity,
            commands::set_process_efficiency,
            commands::is_elevated,
            commands::restart_as_admin,
            commands::close_tcp_connection,
            commands::get_file_lockers,
            commands::list_services,
            commands::control_service,
            commands::list_windows,
            commands::show_window,
            commands::control_window,
            commands::restart_service,
            commands::set_service_start_type,
            commands::get_process_metadata,
            commands::get_process_integrity,
            commands::list_process_modules,
            commands::list_startup_items,
            commands::set_startup_item_enabled,
            commands::delete_startup_item,
            commands::list_drivers,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
