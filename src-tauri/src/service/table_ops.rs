//! 删除、复制、清空、重命名、维护表,信息面板。逻辑都在 core 的 `table_ops`、`table_info` 里;
//! 转储 SQL 文件和导出一样是有进度、能取消的任务,在 [`super::transfer`] 里。

use basalt_core::table::TableRef;
use basalt_core::table_info::TableDetails;
use basalt_core::table_ops::maintenance::{Maintenance, MaintenanceOutcome};
use basalt_core::table_ops::references::ReferencingKey;
use basalt_core::table_ops::{ClearMode, ClearOutcome, CopyOutcome};

use super::CommandResult;
use crate::state::AppState;

pub async fn drop_table(state: &AppState, id: &str, table: &TableRef) -> CommandResult<()> {
    let session = state.sessions.get(id).await?;
    Ok(session.drop_table(table).await?)
}

/// 复制到同一个库(schema)里,新表叫 `name`;`with_data` 为假时只复制结构。
pub async fn copy_table(
    state: &AppState,
    id: &str,
    table: &TableRef,
    name: &str,
    with_data: bool,
) -> CommandResult<CopyOutcome> {
    let session = state.sessions.get(id).await?;
    Ok(session.copy_table(table, name, with_data).await?)
}

/// 清空(DELETE)或截断(TRUNCATE)。
pub async fn clear_table(
    state: &AppState,
    id: &str,
    table: &TableRef,
    mode: ClearMode,
) -> CommandResult<ClearOutcome> {
    let session = state.sessions.get(id).await?;
    Ok(session.clear_table(table, mode).await?)
}

/// 改名,返回新表。
pub async fn rename_table(
    state: &AppState,
    id: &str,
    table: &TableRef,
    name: &str,
) -> CommandResult<TableRef> {
    let session = state.sessions.get(id).await?;
    Ok(session.rename_table(table, name).await?)
}

/// 别的表上引用这张表的外键:清空、截断的确认框要写明会波及哪些表。
pub async fn table_references(
    state: &AppState,
    id: &str,
    table: &TableRef,
) -> CommandResult<Vec<ReferencingKey>> {
    let session = state.sessions.get(id).await?;
    Ok(session.referencing_keys(table).await?)
}

pub async fn maintain_table(
    state: &AppState,
    id: &str,
    table: &TableRef,
    operation: Maintenance,
) -> CommandResult<MaintenanceOutcome> {
    let session = state.sessions.get(id).await?;
    Ok(session.maintain_table(table, operation).await?)
}

/// 信息面板:属性和建表语句。
pub async fn table_details(
    state: &AppState,
    id: &str,
    table: &TableRef,
) -> CommandResult<TableDetails> {
    let session = state.sessions.get(id).await?;
    Ok(session.table_details(table).await?)
}
