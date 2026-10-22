//! 前端可调用的命令:只取出状态,交给 [`crate::service`]。

use basalt_core::config::{ConnectionConfig, ConnectionDraft};
use basalt_core::design::{DdlOutcome, DdlPreview, TableDesign};
use basalt_core::driver::{ServerInfo, TableInfo};
use basalt_core::import::ImportCandidate;
use basalt_core::import::navicat::NavicatLocation;
use basalt_core::import::navicat::queries::{QueryImportCandidate, QueryImportReport};
use basalt_core::llm::ChatRequest;
use basalt_core::llm::client::TestReply;
use basalt_core::llm::config::{LlmModel, LlmModelDraft, LlmSettings};
use basalt_core::llm::outline::OutlineTable;
use basalt_core::query::history::HistoryEntry;
use basalt_core::query::saved::{SavedQueryInfo, SavedQueryRef};
use basalt_core::query::{QueryScope, StatementOutcome};
use basalt_core::table::{ApplyOutcome, ColumnInfo, PageRequest, RowChange, TablePage, TableRef};
use basalt_core::table_info::TableDetails;
use basalt_core::table_ops::maintenance::{Maintenance, MaintenanceOutcome};
use basalt_core::table_ops::references::ReferencingKey;
use basalt_core::table_ops::{ClearMode, ClearOutcome, CopyOutcome};
use basalt_core::transfer::dump::DumpRequest;
use basalt_core::transfer::export::{ExportOutcome, ExportRequest};
use basalt_core::transfer::import::{self as transfer_import, ImportPreview, ImportRequest};
use basalt_core::transfer::progress::Progress;
use basalt_core::transfer::report::ImportFailure;
use basalt_core::transfer::source::SourceSpec;
use basalt_core::tunnel::HostKey;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

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
pub async fn delete_connection(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<service::DeletedConnection> {
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
pub async fn remember_ssh_secret(
    state: State<'_, AppState>,
    id: String,
    secret: String,
) -> CommandResult<()> {
    service::remember_ssh_secret(&state, &id, secret).await
}

#[tauri::command]
pub async fn trust_host_key(state: State<'_, AppState>, host_key: HostKey) -> CommandResult<()> {
    service::trust_host_key(&state, &host_key).await
}

#[tauri::command]
pub async fn open_connection(
    state: State<'_, AppState>,
    id: String,
    password: Option<String>,
    ssh_secret: Option<String>,
) -> CommandResult<ServerInfo> {
    service::open_connection(&state, &id, password, ssh_secret).await
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

#[tauri::command]
pub async fn read_table_design(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
) -> CommandResult<TableDesign> {
    service::read_table_design(&state, &id, &table).await
}

#[tauri::command]
pub async fn preview_table_design(
    state: State<'_, AppState>,
    id: String,
    original: Option<TableRef>,
    design: TableDesign,
) -> CommandResult<DdlPreview> {
    service::preview_table_design(&state, &id, original.as_ref(), &design).await
}

#[tauri::command]
pub async fn apply_table_design(
    state: State<'_, AppState>,
    id: String,
    original: Option<TableRef>,
    design: TableDesign,
    expected: Vec<String>,
) -> CommandResult<DdlOutcome> {
    service::apply_table_design(&state, &id, original.as_ref(), &design, &expected).await
}

#[tauri::command]
pub async fn drop_table(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
) -> CommandResult<()> {
    service::drop_table(&state, &id, &table).await
}

#[tauri::command]
pub async fn copy_table(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
    name: String,
    with_data: bool,
) -> CommandResult<CopyOutcome> {
    service::copy_table(&state, &id, &table, &name, with_data).await
}

#[tauri::command]
pub async fn clear_table(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
    mode: ClearMode,
) -> CommandResult<ClearOutcome> {
    service::clear_table(&state, &id, &table, mode).await
}

#[tauri::command]
pub async fn rename_table(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
    name: String,
) -> CommandResult<TableRef> {
    service::rename_table(&state, &id, &table, &name).await
}

#[tauri::command]
pub async fn table_references(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
) -> CommandResult<Vec<ReferencingKey>> {
    service::table_references(&state, &id, &table).await
}

#[tauri::command]
pub async fn maintain_table(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
    operation: Maintenance,
) -> CommandResult<MaintenanceOutcome> {
    service::maintain_table(&state, &id, &table, operation).await
}

#[tauri::command]
pub async fn table_details(
    state: State<'_, AppState>,
    id: String,
    table: TableRef,
) -> CommandResult<TableDetails> {
    service::table_details(&state, &id, &table).await
}

#[tauri::command]
pub async fn list_llm_models(state: State<'_, AppState>) -> CommandResult<LlmSettings> {
    service::llm::list_llm_models(&state).await
}

#[tauri::command]
pub async fn save_llm_model(
    state: State<'_, AppState>,
    draft: LlmModelDraft,
) -> CommandResult<LlmModel> {
    service::llm::save_llm_model(&state, draft).await
}

#[tauri::command]
pub async fn delete_llm_model(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<LlmSettings> {
    service::llm::delete_llm_model(&state, &id).await
}

#[tauri::command]
pub async fn set_active_llm_model(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<LlmSettings> {
    service::llm::set_active_llm_model(&state, &id).await
}

#[tauri::command]
pub async fn test_llm_model(
    state: State<'_, AppState>,
    draft: LlmModelDraft,
) -> CommandResult<TestReply> {
    service::llm::test_llm_model(&state, draft).await
}

/// 助手回答的流式片段,事件名和前端 api.ts 一致。
const LLM_CHAT_EVENT: &str = "llm-chat";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatDelta<'a> {
    request_id: &'a str,
    text: &'a str,
}

#[tauri::command]
pub async fn llm_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    request_id: String,
    request: ChatRequest,
) -> CommandResult<service::llm::ChatOutcome> {
    let id = request_id.clone();
    let on_text = move |text: &str| {
        let payload = ChatDelta {
            request_id: &id,
            text,
        };
        if let Err(error) = app.emit(LLM_CHAT_EVENT, payload) {
            log::warn!("发送助手回答失败：{error}");
        }
    };
    service::llm::llm_chat(&state, &request_id, request, on_text).await
}

#[tauri::command]
pub async fn cancel_llm_chat(
    state: State<'_, AppState>,
    request_id: String,
) -> CommandResult<bool> {
    Ok(service::llm::cancel_llm_chat(&state, &request_id))
}

#[tauri::command]
pub async fn schema_outline(
    state: State<'_, AppState>,
    id: String,
    database: String,
    schema: Option<String>,
) -> CommandResult<Vec<OutlineTable>> {
    service::llm::schema_outline(&state, &id, &database, schema.as_deref()).await
}

/// 本机 Navicat 的配置在用户主目录下;找不到主目录只影响本机来源,不妨碍导入 .ncx。
/// 只认 macOS 版的位置:Windows 版存在注册表里,见 `navicat::local_unavailable`。
fn local_navicat(app: &AppHandle) -> Option<NavicatLocation> {
    if !cfg!(target_os = "macos") {
        return None;
    }
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

/// 上次开着的标签页;第一次启动、或记录读不懂时为空。
#[tauri::command]
pub async fn load_workspace(state: State<'_, AppState>) -> CommandResult<Option<String>> {
    Ok(state.workspace.load()?)
}

/// 记下现在开着的标签页,前端在内容变了之后稍等一会儿调用。
#[tauri::command]
pub async fn save_workspace(state: State<'_, AppState>, content: String) -> CommandResult<()> {
    Ok(state.workspace.save(&content)?)
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

/// 导入导出的进度事件名,和前端 api.ts 一致。
const TRANSFER_PROGRESS_EVENT: &str = "transfer-progress";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgress<'a> {
    job_id: &'a str,
    #[serde(flatten)]
    progress: &'a Progress,
}

/// 进度经 Tauri 事件推给前端,前端按任务 id 认领。
fn progress_events(app: AppHandle, job_id: String) -> impl Fn(&Progress) + Send + Sync {
    move |progress| {
        let payload = TransferProgress {
            job_id: &job_id,
            progress,
        };
        if let Err(error) = app.emit(TRANSFER_PROGRESS_EVENT, payload) {
            log::warn!("发送导入导出进度失败：{error}");
        }
    }
}

#[tauri::command]
pub async fn export_data(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
    request: ExportRequest,
) -> CommandResult<ExportOutcome> {
    let progress = progress_events(app, job_id.clone());
    service::transfer::export_data(&state, &job_id, &request, &progress).await
}

#[tauri::command]
pub async fn dump_table(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
    request: DumpRequest,
) -> CommandResult<ExportOutcome> {
    let progress = progress_events(app, job_id.clone());
    service::transfer::dump_table(&state, &job_id, &request, &progress).await
}

#[tauri::command]
pub async fn preview_import(
    state: State<'_, AppState>,
    connection_id: String,
    table: TableRef,
    source: SourceSpec,
) -> CommandResult<ImportPreview> {
    service::transfer::preview_import_file(&state, &connection_id, &table, source).await
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
    request: ImportRequest,
) -> CommandResult<transfer_import::ImportReport> {
    let progress = progress_events(app, job_id.clone());
    service::transfer::import_data(&state, &job_id, &request, &progress).await
}

#[tauri::command]
pub async fn cancel_transfer(state: State<'_, AppState>, job_id: String) -> CommandResult<bool> {
    service::transfer::cancel_transfer(&state, &job_id).await
}

#[tauri::command]
pub async fn save_import_failures(
    path: String,
    failures: Vec<ImportFailure>,
) -> CommandResult<u64> {
    service::transfer::save_import_failures(&path, &failures)
}

#[tauri::command]
pub async fn reveal_in_finder(path: String) -> CommandResult<()> {
    service::transfer::reveal_in_finder(&path)
}
