//! SQLite 查询标签页的连接。值按各自的存储类型解码,和表数据一样。
//!
//! 取消:执行期间装一个 progress handler,每隔一批虚拟机指令看一眼取消标志,
//! 标志一亮就让 SQLite 中止当前语句(返回 SQLITE_INTERRUPT,效果与 sqlite3_interrupt 相同)。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sqlx::sqlite::SqliteConnection;
use sqlx::{Connection, Executor};

use super::sqlite::SqliteSession;
use super::sqlite_data::decode_cell;
use crate::error::CoreResult;
use crate::query::ResultColumn;
use crate::query::collect::{Collected, collect, described_columns};
use crate::sql::dynamic;

/// 每执行这么多条虚拟机指令检查一次取消标志:够及时,又几乎不拖慢执行。
pub(super) const PROGRESS_STEPS: i32 = 1000;

pub(crate) struct SqliteEditor {
    pub(super) conn: SqliteConnection,
}

impl SqliteSession {
    pub(crate) async fn open_editor(&self) -> CoreResult<SqliteEditor> {
        Ok(SqliteEditor {
            conn: self.pool.acquire().await?.detach(),
        })
    }
}

impl SqliteEditor {
    pub(crate) async fn run(&mut self, sql: &str, cancelled: Arc<AtomicBool>) -> Collected {
        match self.conn.lock_handle().await {
            Ok(mut handle) => handle
                .set_progress_handler(PROGRESS_STEPS, move || !cancelled.load(Ordering::Relaxed)),
            Err(error) => {
                return Collected {
                    result_sets: Vec::new(),
                    affected: 0,
                    error: Some(error),
                };
            }
        }
        let stream = sqlx::raw_sql(dynamic(sql)).fetch_many(&mut self.conn);
        let collected = collect(
            stream,
            decode_cell,
            sqlx::sqlite::SqliteQueryResult::rows_affected,
        )
        .await;
        match self.conn.lock_handle().await {
            Ok(mut handle) => handle.remove_progress_handler(),
            Err(error) => log::warn!("移除 SQLite progress handler 失败：{error}"),
        }
        collected
    }

    pub(crate) async fn describe(&mut self, sql: &str) -> Vec<ResultColumn> {
        described_columns((&mut self.conn).prepare(dynamic(sql)).await)
    }

    pub(crate) async fn ping(&mut self) -> bool {
        self.conn.ping().await.is_ok()
    }

    pub(crate) async fn clear_cached_statements(&mut self) -> Result<(), sqlx::Error> {
        self.conn.clear_cached_statements().await
    }

    pub(crate) async fn close(self) {
        if let Err(error) = self.conn.close().await {
            log::warn!("关闭查询标签页的 SQLite 连接失败：{error}");
        }
    }
}
