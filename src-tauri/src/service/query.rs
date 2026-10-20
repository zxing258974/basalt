//! SQL 编辑器的命令:执行与取消、补全用的列信息、执行历史、保存的查询。

use basalt_core::query::history::HistoryEntry;
use basalt_core::query::saved::{SavedQueryInfo, SavedQueryRef};
use basalt_core::query::{QueryScope, StatementOutcome};
use basalt_core::table::{ColumnInfo, TableRef};

use super::CommandResult;
use crate::state::AppState;

/// 历史面板一次最多给多少条。
pub const MAX_HISTORY_RESULTS: usize = 500;

/// 在查询标签页 `editor_id` 的专用连接上执行一条语句,并记进历史。
/// 记历史失败只留日志:语句已经执行了,不能因此报错。
pub async fn run_statement(
    state: &AppState,
    editor_id: &str,
    connection_id: &str,
    scope: &QueryScope,
    sql: &str,
) -> CommandResult<StatementOutcome> {
    let outcome = state
        .sessions
        .run_query(editor_id, connection_id, scope, sql)
        .await?;
    let connection_name = state
        .find(connection_id)
        .map_or_else(|_| connection_id.to_owned(), |config| config.name);
    let entry = HistoryEntry::record(connection_id, &connection_name, scope, sql, &outcome);
    if let Err(error) = state.history.append(&entry) {
        log::warn!("记录执行历史失败：{error}");
    }
    Ok(outcome)
}

pub async fn cancel_statement(state: &AppState, editor_id: &str) -> CommandResult<bool> {
    Ok(state.sessions.cancel_query(editor_id).await?)
}

pub async fn close_editor(state: &AppState, editor_id: &str) {
    state.sessions.close_editor(editor_id).await;
}

/// 补全要的列名:复用表数据的列信息查询。
pub async fn list_columns(
    state: &AppState,
    id: &str,
    table: &TableRef,
) -> CommandResult<Vec<ColumnInfo>> {
    let session = state.sessions.get(id).await?;
    Ok(session.table_columns(table).await?)
}

pub fn search_history(
    state: &AppState,
    query: &str,
    connection_id: Option<&str>,
    limit: usize,
) -> CommandResult<Vec<HistoryEntry>> {
    Ok(state
        .history
        .search(query, connection_id, limit.min(MAX_HISTORY_RESULTS))?)
}

pub fn clear_history(state: &AppState) -> CommandResult<()> {
    Ok(state.history.clear()?)
}

pub fn list_saved_queries(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: Option<&str>,
) -> CommandResult<Vec<SavedQueryInfo>> {
    Ok(state.saved_queries.list(connection_id, database, schema)?)
}

pub fn read_saved_query(state: &AppState, query: &SavedQueryRef) -> CommandResult<String> {
    Ok(state.saved_queries.read(query)?)
}

pub fn save_query(
    state: &AppState,
    query: &SavedQueryRef,
    sql: &str,
    overwrite: bool,
) -> CommandResult<SavedQueryInfo> {
    Ok(state.saved_queries.save(query, sql, overwrite)?)
}

pub fn delete_saved_query(state: &AppState, query: &SavedQueryRef) -> CommandResult<()> {
    Ok(state.saved_queries.delete(query)?)
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
