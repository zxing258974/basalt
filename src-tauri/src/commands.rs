//! 前端可调用的命令:只取出状态,交给 [`crate::service`]。

use basalt_core::config::{ConnectionConfig, ConnectionDraft};
use basalt_core::driver::{ServerInfo, TableInfo};
use basalt_core::import::ImportCandidate;
use basalt_core::import::navicat::NavicatLocation;
use tauri::{AppHandle, Manager, State};

use crate::service::{self, CommandResult, ConnectionView, ImportReport, ImportSource};
use crate::state::AppState;

#[tauri::command]
pub async fn list_connections(state: State<'_, AppState>) -> CommandResult<Vec<ConnectionView>> {
    service::list_connections(&state).await
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    draft: ConnectionDraft,
) -> CommandResult<ConnectionConfig> {
    service::save_connection(&state, draft).await
}

#[tauri::command]
pub async fn remember_password(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CommandResult<()> {
    service::remember_password(&state, &id, password).await
}

#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    service::delete_connection(&state, &id).await
}

#[tauri::command]
pub async fn duplicate_connection(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<ConnectionConfig> {
    service::duplicate_connection(&state, &id).await
}

#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    draft: ConnectionDraft,
) -> CommandResult<ServerInfo> {
    service::test_connection(&state, draft).await
}

#[tauri::command]
pub async fn open_connection(
    state: State<'_, AppState>,
    id: String,
    password: Option<String>,
) -> CommandResult<ServerInfo> {
    service::open_connection(&state, &id, password).await
}

#[tauri::command]
pub async fn close_connection(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    service::close_connection(&state, &id).await;
    Ok(())
}

#[tauri::command]
pub async fn list_databases(state: State<'_, AppState>, id: String) -> CommandResult<Vec<String>> {
    service::list_databases(&state, &id).await
}

#[tauri::command]
pub async fn list_schemas(
    state: State<'_, AppState>,
    id: String,
    database: String,
) -> CommandResult<Vec<String>> {
    service::list_schemas(&state, &id, &database).await
}

#[tauri::command]
pub async fn list_tables(
    state: State<'_, AppState>,
    id: String,
    database: String,
    schema: Option<String>,
) -> CommandResult<Vec<TableInfo>> {
    service::list_tables(&state, &id, &database, schema.as_deref()).await
}

/// 本机 Navicat 的配置在用户主目录下。
fn local_navicat(app: &AppHandle) -> CommandResult<NavicatLocation> {
    let home = app.path().home_dir().map_err(service::home_error)?;
    Ok(NavicatLocation::under_home(&home))
}

#[tauri::command]
pub async fn scan_import_source(
    app: AppHandle,
    state: State<'_, AppState>,
    source: ImportSource,
) -> CommandResult<Vec<ImportCandidate>> {
    service::scan_import_source(&state, &source, &local_navicat(&app)?).await
}

#[tauri::command]
pub async fn import_connections(
    app: AppHandle,
    state: State<'_, AppState>,
    source: ImportSource,
    ids: Vec<String>,
) -> CommandResult<ImportReport> {
    service::import_connections(&state, &source, &ids, &local_navicat(&app)?).await
}
