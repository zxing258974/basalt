//! 删除表、复制表、信息面板。逻辑都在 core 的 `table_ops`、`table_info` 里。

use basalt_core::table::TableRef;
use basalt_core::table_info::TableDetails;
use basalt_core::table_ops::CopyOutcome;

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

/// 信息面板:属性和建表语句。
pub async fn table_details(
    state: &AppState,
    id: &str,
    table: &TableRef,
) -> CommandResult<TableDetails> {
    let session = state.sessions.get(id).await?;
    Ok(session.table_details(table).await?)
}
