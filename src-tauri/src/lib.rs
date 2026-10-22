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

use basalt_core::secret::{FileSecretStore, KeychainStore, SecretStore};
use tauri::Manager;

/// 以前存密码的钥匙串服务名:只在把旧密码搬进 secrets.json 时读。
const KEYCHAIN_SERVICE: &str = "dev.basalt.app";

/// 密码和 API Key 放哪儿:配置目录的 secrets.json,只有当前用户能读写。不用钥匙串,也就不弹授权框。
/// 以前存在钥匙串里的(macOS 合成的那一条和更早的单条;Windows 凭据管理器)启动时一次只读地搬过来。
fn secret_store(config_dir: &std::path::Path) -> Arc<FileSecretStore> {
    let legacy = KeychainStore::new(KEYCHAIN_SERVICE);
    #[cfg(target_os = "macos")]
    let legacy = basalt_core::secret::VaultSecretStore::new(
        legacy,
        config_dir.join("pending-secret-deletes.json"),
    )
    .read_only();
    Arc::new(FileSecretStore::new(config_dir.join("secrets.json")).with_legacy(Box::new(legacy)))
}

/// 把钥匙串里的旧密码一次搬完:授权框都在启动时弹,之后再也不碰钥匙串。
/// 读钥匙串会阻塞到用户点完授权框,放到后台线程;先打开的连接要密码时等这把锁,不会再弹一次。
fn migrate_legacy_secrets(state: &state::AppState, secrets: Arc<FileSecretStore>) {
    let keys = match state.saved_secret_keys() {
        Ok(keys) => keys,
        Err(error) => {
            log::warn!("读配置失败，这次不搬钥匙串里的旧密码：{error}");
            return;
        }
    };
    tauri::async_runtime::spawn_blocking(move || match secrets.migrate_legacy(&keys) {
        Ok(0) => {}
        Ok(moved) => log::info!("从钥匙串搬过来 {moved} 条密码"),
        Err(error) => log::warn!("从钥匙串搬旧密码被拒绝，剩下的不搬了、以后也不再问：{error}"),
    });
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
            let secrets = secret_store(&config_dir);
            let state = state::AppState::new(
                config_dir.join("connections.json"),
                secrets.clone() as Arc<dyn SecretStore>,
            );
            migrate_legacy_secrets(&state, secrets);
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::remember_password,
            commands::remember_ssh_secret,
            commands::trust_host_key,
            commands::list_ssh_config_hosts,
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
            commands::table_references,
            commands::table_details,
            commands::server_snapshot,
            commands::kill_process,
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
            commands::result_rows,
            commands::release_results,
            commands::close_editor,
            commands::list_columns,
            commands::search_history,
            commands::clear_history,
            commands::list_saved_queries,
            commands::read_saved_query,
            commands::save_query,
            commands::load_workspace,
            commands::save_workspace,
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
