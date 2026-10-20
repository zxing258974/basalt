mod commands;
mod service;
#[cfg(test)]
mod service_tests;
mod state;

use std::sync::Arc;

use basalt_core::secret::KeychainStore;
use tauri::Manager;

/// 钥匙串里的服务名。
const KEYCHAIN_SERVICE: &str = "dev.basalt.app";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            let config_path = app.path().app_config_dir()?.join("connections.json");
            app.manage(state::AppState::new(
                config_path,
                Arc::new(KeychainStore::new(KEYCHAIN_SERVICE)),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::remember_password,
            commands::delete_connection,
            commands::duplicate_connection,
            commands::test_connection,
            commands::open_connection,
            commands::close_connection,
            commands::list_databases,
            commands::list_schemas,
            commands::list_tables,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Basalt");
}
