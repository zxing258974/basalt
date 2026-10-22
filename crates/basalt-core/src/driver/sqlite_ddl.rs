//! SQLite 的 DDL 执行:一个事务里执行;重建表时按官方流程在事务外关掉外键约束,
//! 提交前做外键检查,结束后恢复连接原来的设置。

use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::{Connection, Row};

use super::sqlite::SqliteSession;
use super::{finish_transaction, statement_failed};
use crate::design::{DdlStatement, StatementKind};
use crate::error::{CoreError, CoreResult};
use crate::sql::dynamic;

impl SqliteSession {
    /// 在一个事务里执行。`rebuild` 时按官方流程:事务外关掉外键约束,结束后恢复。
    pub async fn execute_ddl(
        &self,
        statements: &[DdlStatement],
        rebuild: bool,
    ) -> CoreResult<usize> {
        let mut connection = self.pool.acquire().await?;
        let foreign_keys = rebuild
            && sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                .fetch_one(&mut *connection)
                .await?
                == 1;
        if foreign_keys {
            sqlx::query("PRAGMA foreign_keys = OFF")
                .execute(&mut *connection)
                .await?;
        }
        let result = run(&mut connection, statements).await;
        // 失败时 legacy_alter_table 可能还开着;这些是连接级设置,不随事务回滚。
        let restore = async {
            sqlx::query("PRAGMA legacy_alter_table = OFF")
                .execute(&mut *connection)
                .await?;
            if foreign_keys {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *connection)
                    .await?;
            }
            Ok::<_, sqlx::Error>(())
        }
        .await;
        if let Err(error) = restore {
            log::warn!("恢复 SQLite 连接设置失败，丢弃这条连接：{error}");
            drop(connection.detach());
        }
        result
    }
}

async fn run(connection: &mut SqliteConnection, statements: &[DdlStatement]) -> CoreResult<usize> {
    let mut transaction = connection.begin().await?;
    let result = async {
        for (index, statement) in statements.iter().enumerate() {
            let query = sqlx::query(dynamic(&statement.sql)).persistent(false);
            match statement.kind {
                StatementKind::Execute => {
                    query
                        .execute(&mut *transaction)
                        .await
                        .map_err(|error| statement_failed(index, error))?;
                }
                StatementKind::ForeignKeyCheck => {
                    let rows = query
                        .fetch_all(&mut *transaction)
                        .await
                        .map_err(|error| statement_failed(index, error))?;
                    if let Some(first) = rows.first() {
                        return Err(foreign_key_violation(first, rows.len()));
                    }
                }
            }
        }
        Ok(statements.len() as u64)
    }
    .await;
    Ok(finish_transaction(transaction, result).await? as usize)
}

fn foreign_key_violation(row: &SqliteRow, count: usize) -> CoreError {
    let table: String = row.try_get("table").unwrap_or_default();
    let parent: String = row.try_get("parent").unwrap_or_default();
    let rowid = row
        .try_get::<Option<i64>, _>("rowid")
        .ok()
        .flatten()
        .map_or(String::new(), |id| format!("（如 rowid {id}）"));
    CoreError::InvalidEdit(format!(
        "外键检查没通过：表「{table}」有 {count} 行引用的记录在「{parent}」里找不到{rowid}。所有修改都已回滚。"
    ))
}
