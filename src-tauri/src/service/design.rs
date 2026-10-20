//! 表结构设计:读结构、预览 DDL、执行 DDL。逻辑都在 core 的 `design` 里。

use basalt_core::design::{DdlOutcome, DdlPreview, TableDesign};
use basalt_core::table::TableRef;

use super::CommandResult;
use crate::state::AppState;

pub async fn read_table_design(
    state: &AppState,
    id: &str,
    table: &TableRef,
) -> CommandResult<TableDesign> {
    let session = state.sessions.get(id).await?;
    Ok(session.read_design(table).await?)
}

/// 将要执行的 DDL,前端在执行前给用户确认。`original` 为 `None` 表示新建表。
pub async fn preview_table_design(
    state: &AppState,
    id: &str,
    original: Option<&TableRef>,
    design: &TableDesign,
) -> CommandResult<DdlPreview> {
    let session = state.sessions.get(id).await?;
    Ok(session.preview_design(original, design).await?)
}

/// `expected` 是用户确认过的预览,后端重新生成的不一样就拒绝执行。
pub async fn apply_table_design(
    state: &AppState,
    id: &str,
    original: Option<&TableRef>,
    design: &TableDesign,
    expected: &[String],
) -> CommandResult<DdlOutcome> {
    let session = state.sessions.get(id).await?;
    Ok(session
        .apply_design(original, design, Some(expected))
        .await?)
}

#[cfg(test)]
#[path = "design_tests.rs"]
mod tests;
