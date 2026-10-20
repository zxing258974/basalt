//! 前端可调用的命令:只取出状态,交给 [`crate::service`]。

use basalt_core::config::{ConnectionConfig, ConnectionDraft};
use basalt_core::driver::{ServerInfo, TableInfo};
use basalt_core::import::ImportCandidate;
use basalt_core::import::navicat::NavicatLocation;
use basalt_core::import::navicat::queries::{QueryImportCandidate, QueryImportReport};
use basalt_core::query::history::HistoryEntry;
use basalt_core::query::saved::{SavedQueryInfo, SavedQueryRef};
use basalt_core::query::{QueryScope, StatementOutcome};
use basalt_core::table::{ApplyOutcome, ColumnInfo, PageRequest, RowChange, TablePage, TableRef};
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

#[tauri::command]
pub async fn read_table_page(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
    request: PageRequest,
) -> CommandResult<TablePage> {
    service::read_table_page(&state, &id, &table, &request).await
}

#[tauri::command]
pub async fn preview_table_changes(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
    changes: Vec<RowChange>,
) -> CommandResult<Vec<String>> {
    service::preview_table_changes(&state, &id, &table, &changes).await
}

#[tauri::command]
pub async fn apply_table_changes(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
    changes: Vec<RowChange>,
    expected: Vec<String>,
) -> CommandResult<ApplyOutcome> {
    service::apply_table_changes(&state, &id, &table, &changes, &expected).await
}

/// 本机 Navicat 的配置在用户主目录下;找不到主目录只影响本机来源,不妨碍导入 .ncx。
fn local_navicat(app: &AppHandle) -> Option<NavicatLocation> {
    let home = app.path().home_dir().ok()?;
    Some(NavicatLocation::under_home(&home))
}

#[tauri::command]
pub async fn scan_import_source(
    app: AppHandle,
    state: State<'_, AppState>,
    source: ImportSource,
) -> CommandResult<Vec<ImportCandidate>> {
    service::scan_import_source(&state, &source, local_navicat(&app).as_ref()).await
}

#[tauri::command]
pub async fn import_connections(
    app: AppHandle,
    state: State<'_, AppState>,
    source: ImportSource,
    ids: Vec<String>,
) -> CommandResult<ImportReport> {
    service::import_connections(&state, &source, &ids, local_navicat(&app).as_ref()).await
}

#[tauri::command]
pub async fn run_statement(
    state: State<'_, AppState>,
    editor_id: String,
    connection_id: String,
    scope: QueryScope,
    sql: String,
) -> CommandResult<StatementOutcome> {
    service::query::run_statement(&state, &editor_id, &connection_id, &scope, &sql).await
}

#[tauri::command]
pub async fn cancel_statement(
    state: State<'_, AppState>,
    editor_id: String,
) -> CommandResult<bool> {
    service::query::cancel_statement(&state, &editor_id).await
}

#[tauri::command]
pub async fn close_editor(state: State<'_, AppState>, editor_id: String) -> CommandResult<()> {
    service::query::close_editor(&state, &editor_id).await;
    Ok(())
}

#[tauri::command]
pub async fn list_columns(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
) -> CommandResult<Vec<ColumnInfo>> {
    service::query::list_columns(&state, &id, &table).await
}

#[tauri::command]
pub async fn search_history(
    state: State<'_, AppState>,
    query: String,
    connection_id: Option<String>,
    limit: usize,
) -> CommandResult<Vec<HistoryEntry>> {
    service::query::search_history(&state, &query, connection_id.as_deref(), limit)
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AppState>) -> CommandResult<()> {
    service::query::clear_history(&state)
}

#[tauri::command]
pub async fn list_saved_queries(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    schema: Option<String>,
) -> CommandResult<Vec<SavedQueryInfo>> {
    service::query::list_saved_queries(&state, &connection_id, &database, schema.as_deref())
}

#[tauri::command]
pub async fn read_saved_query(
    state: State<'_, AppState>,
    query: SavedQueryRef,
) -> CommandResult<String> {
    service::query::read_saved_query(&state, &query)
}

#[tauri::command]
pub async fn save_query(
    state: State<'_, AppState>,
    query: SavedQueryRef,
    sql: String,
    overwrite: bool,
) -> CommandResult<SavedQueryInfo> {
    service::query::save_query(&state, &query, &sql, overwrite)
}

#[tauri::command]
pub async fn delete_saved_query(
    state: State<'_, AppState>,
    query: SavedQueryRef,
) -> CommandResult<()> {
    service::query::delete_saved_query(&state, &query)
}

#[tauri::command]
pub async fn scan_navicat_queries(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Vec<QueryImportCandidate>> {
    service::scan_navicat_queries(&state, local_navicat(&app).as_ref()).await
}

#[tauri::command]
pub async fn import_navicat_queries(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> CommandResult<QueryImportReport> {
    service::import_navicat_queries(&state, &ids, local_navicat(&app).as_ref()).await
}
