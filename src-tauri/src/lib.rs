mod commands;
mod quit;
mod service;
#[cfg(test)]
mod service_table_tests;
#[cfg(test)]
mod service_tests;
mod state;
#[cfg(windows)]
mod webview_windows;

use std::sync::Arc;

use basalt_core::secret::{CachedSecretStore, KeychainStore, SecretStore};
use tauri::Manager;

/// 钥匙串里的服务名。
const KEYCHAIN_SERVICE: &str = "dev.basalt.app";

/// 密码和 API Key 放哪儿。两边都在内存里缓存读到的结果:同一条一次运行只读一次,不会每发一句话都弹框。
/// macOS 上全部合成钥匙串里的一条:包是临时签名,每次重装都要重新放行,条目越少问得越少。
/// Windows 凭据管理器单条有长度上限,还是一条连接一条(那边也不会弹框)。
fn secret_store(config_dir: &std::path::Path) -> Arc<dyn SecretStore> {
    let keychain = KeychainStore::new(KEYCHAIN_SERVICE);
    #[cfg(target_os = "macos")]
    let keychain = basalt_core::secret::VaultSecretStore::new(
        keychain,
        config_dir.join("pending-secret-deletes.json"),
    );
    #[cfg(not(target_os = "macos"))]
    let _ = config_dir;
    Arc::new(CachedSecretStore::new(keychain))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    // 菜单只在 macOS 上装:Windows 上它会变成窗口里的一条 File/Edit 菜单栏,系统自带的「Quit」还会绕过退出确认。
    #[cfg(target_os = "macos")]
    let builder = builder.menu(quit::menu).on_menu_event(quit::on_menu_event);
    builder
        .on_window_event(quit::on_window_event)
        .manage(quit::QuitGuard::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            // 开发时留着 F5 刷新,打出来的包才关。
            #[cfg(windows)]
            if !cfg!(debug_assertions)
                && let Some(window) = app.get_webview_window("main")
            {
                webview_windows::disable_browser_shortcuts(&window);
            }
            let config_dir = app.path().app_config_dir()?;
            app.manage(state::AppState::new(
                config_dir.join("connections.json"),
                secret_store(&config_dir),
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
            commands::read_table_page,
            commands::preview_table_changes,
            commands::apply_table_changes,
            commands::read_table_design,
            commands::preview_table_design,
            commands::apply_table_design,
            commands::drop_table,
            commands::copy_table,
            commands::clear_table,
            commands::rename_table,
            commands::maintain_table,
            commands::table_details,
            commands::list_llm_models,
            commands::save_llm_model,
            commands::delete_llm_model,
            commands::set_active_llm_model,
            commands::test_llm_model,
            commands::llm_chat,
            commands::cancel_llm_chat,
            commands::schema_outline,
            commands::scan_import_source,
            commands::import_connections,
            commands::run_statement,
            commands::cancel_statement,
            commands::close_editor,
            commands::list_columns,
            commands::search_history,
            commands::clear_history,
            commands::list_saved_queries,
            commands::read_saved_query,
            commands::save_query,
            commands::delete_saved_query,
            commands::scan_navicat_queries,
            commands::import_navicat_queries,
            commands::export_data,
            commands::dump_table,
            commands::preview_import,
            commands::import_data,
            commands::cancel_transfer,
            commands::save_import_failures,
            commands::reveal_in_finder,
            quit::set_unsaved_work,
            quit::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Basalt");
}
