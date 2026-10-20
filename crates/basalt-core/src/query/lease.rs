//! 导入导出借用一条专用连接。和查询标签页走同一个 [`EditorRegistry`]:取消一样是另开连接
//! KILL QUERY / pg_cancel_backend、SQLite 中断;所属连接关闭时一起断开;同一条连接上的
//! 查询要等借用结束才执行。
//!
//! 借用期间一直持有连接的锁;每条语句执行时登记成「执行中」,两条语句之间登记成「已借出」,
//! 这时来的取消只记下来,由调用方在下一步之前检查。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use tokio::sync::OwnedMutexGuard;

use super::editor::{
    EditorSlot, OpenEditor, Phase, Running, error_text, is_connection_lost, prepare,
};
use super::{EditorRegistry, QueryScope, ResultColumn};
use crate::driver::editor::CancelHandle;
use crate::error::{CoreError, CoreResult};
use crate::session::Session;
use crate::sql::dml::Statement;
use crate::table::ColumnInfo;
use crate::value::CellValue;

/// 流式读出来的行交给谁。
pub trait RowSink: Send {
    /// 用户的 SQL 读到第一行(或者确定没有行)时给出列;表数据的列调用方本来就知道,不调用。
    fn columns(&mut self, columns: &[ResultColumn]) -> CoreResult<()>;
    fn row(&mut self, row: Vec<CellValue>) -> CoreResult<()>;
    /// 每写完一行问一次要不要停下来(用户取消)。
    fn should_stop(&self) -> bool;
}

pub enum StreamQuery<'a> {
    /// 用户写的 SQL:文本协议执行、按结果列类型解码,和查询标签页一样。只取第一个结果集。
    Text(&'a str),
    /// 表数据的 SELECT:预备语句、按表结构的列类型解码,和表数据浏览一样。
    Table {
        sql: &'a str,
        columns: &'a [ColumnInfo],
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamEnd {
    Finished { rows: u64 },
    Cancelled,
}

/// 执行一条语句失败的三种情况,导入时分别处理。
#[derive(Debug, PartialEq, Eq)]
pub enum ExecError {
    /// 用户取消了。
    Cancelled,
    /// 连接断了:这条连接不能再用,后面的也别执行了。
    Lost(String),
    /// 数据库拒绝了这条语句(约束、类型……),连接还能接着用。
    Rejected(String),
}

/// 借来的专用连接,用完调用 [`Self::release`]。
pub struct EditorLease {
    slot: Arc<EditorSlot>,
    state: OwnedMutexGuard<Option<OpenEditor>>,
    session: Arc<Session>,
    handle: CancelHandle,
    requested: Arc<AtomicBool>,
    notice: Option<String>,
    broken: bool,
    released: bool,
}

impl EditorRegistry {
    /// 借出 `editor_id` 的专用连接(没有就新建),切到 `scope`。准备期间被取消返回 [`CoreError::Cancelled`]。
    pub(crate) async fn lease(
        &self,
        editor_id: &str,
        connection_id: &str,
        session: &Arc<Session>,
        scope: &QueryScope,
    ) -> CoreResult<EditorLease> {
        let slot = self.slot(editor_id, connection_id).await;
        let mut state = slot.state.clone().lock_owned().await;
        *slot.phase.lock().await = Phase::Preparing {
            cancel_requested: false,
        };
        let prepared = prepare(&mut state, session, scope).await;
        let handle = match (&prepared, state.as_ref()) {
            (Ok(_), Some(open)) => open.conn.cancel_handle(session).await,
            (Ok(_), None) => Err(CoreError::Superseded),
            (Err(_), _) => Err(CoreError::Superseded),
        };
        let mut phase = slot.phase.lock().await;
        let cancelled = matches!(
            *phase,
            Phase::Preparing {
                cancel_requested: true
            }
        );
        let (notice, handle) = match (prepared, handle) {
            (Err(error), _) | (Ok(_), Err(error)) => {
                *phase = Phase::Idle;
                return Err(error);
            }
            _ if cancelled => {
                *phase = Phase::Idle;
                return Err(CoreError::Cancelled);
            }
            (Ok(notice), Ok(handle)) => (notice, handle),
        };
        let requested = Arc::new(AtomicBool::new(false));
        *phase = Phase::Leased(requested.clone());
        drop(phase);
        Ok(EditorLease {
            slot,
            state,
            session: session.clone(),
            handle,
            requested,
            notice,
            broken: false,
            released: false,
        })
    }
}

impl EditorLease {
    /// 借用期间有没有人要求取消。
    pub fn cancel_requested(&self) -> bool {
        self.requested.load(Ordering::Relaxed)
    }

    /// 准备连接时发生的、用户应该知道的事(断线重连过、换库开了新连接)。
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    fn open(&mut self) -> CoreResult<&mut OpenEditor> {
        self.state.as_mut().ok_or(CoreError::Superseded)
    }

    /// 登记成「执行中」。取消请求已经来了就不登记,返回 false:检查和登记在同一把锁里,
    /// 不会出现「刚检查完没取消、取消就到了、语句照样执行到底」的空当。
    async fn enter(&self) -> bool {
        let mut phase = self.slot.phase.lock().await;
        if self.cancel_requested() {
            return false;
        }
        *phase = Phase::Running(Running {
            handle: self.handle.clone(),
            requested: self.requested.clone(),
        });
        true
    }

    /// 回到「已借出」。取消请求说到一半时要等它说完(它持着这把锁),KILL 才不会落到下一条上。
    async fn leave(&self) {
        *self.slot.phase.lock().await = Phase::Leased(self.requested.clone());
    }

    /// 流式执行一条查询,逐行交给 `sink`。
    pub async fn stream(
        &mut self,
        query: StreamQuery<'_>,
        sink: &mut dyn RowSink,
    ) -> CoreResult<StreamEnd> {
        if !self.enter().await {
            return Ok(StreamEnd::Cancelled);
        }
        let (handle, requested) = (self.handle.clone(), self.requested.clone());
        let driven = match self.open() {
            Ok(open) => {
                let driven = open.conn.stream(&query, &handle, &requested, sink).await;
                open.last_used = Instant::now();
                Ok(driven)
            }
            Err(error) => Err(error),
        };
        self.leave().await;
        let driven = driven?;
        if let Some(stop) = driven.stop {
            return match stop {
                Ok(()) => Ok(StreamEnd::Cancelled),
                Err(error) => Err(error),
            };
        }
        if let Some(error) = driven.error {
            if self.cancel_requested() {
                return Ok(StreamEnd::Cancelled);
            }
            self.broken = is_connection_lost(&error);
            return Err(CoreError::Database(error_text(error)));
        }
        if let StreamQuery::Text(sql) = query
            && !driven.columns_seen
        {
            let columns = self.open()?.conn.describe(sql).await;
            sink.columns(&columns)?;
        }
        Ok(StreamEnd::Finished { rows: driven.rows })
    }

    /// 执行一条参数化语句,返回影响行数。
    pub async fn execute(&mut self, statement: &Statement) -> Result<u64, ExecError> {
        self.step(Step::Statement(statement)).await
    }

    /// 执行一条不带参数的语句(BEGIN、COMMIT、SAVEPOINT、DELETE……)。
    pub async fn execute_sql(&mut self, sql: &str) -> Result<u64, ExecError> {
        self.step(Step::Sql(sql)).await
    }

    /// 收尾用(COMMIT、ROLLBACK):取消之后也要执行;执行期间来的取消只记下来,不去中止它
    /// (被 KILL 的 COMMIT 也可能已经生效,说不清),也不让 SQLite 的取消标志打断它。
    pub async fn cleanup_sql(&mut self, sql: &str) -> Result<u64, ExecError> {
        self.step(Step::Cleanup(sql)).await
    }

    async fn step(&mut self, step: Step<'_>) -> Result<u64, ExecError> {
        let cleanup = matches!(step, Step::Cleanup(_));
        if !cleanup && !self.enter().await {
            return Err(ExecError::Cancelled);
        }
        let handle = self.handle.clone();
        let result = match self.state.as_mut() {
            Some(open) => {
                let result = match step {
                    Step::Statement(statement) => {
                        open.conn.execute_statement(statement, &handle).await
                    }
                    Step::Sql(sql) => open.conn.execute_sql(sql, Some(&handle)).await,
                    Step::Cleanup(sql) => open.conn.execute_sql(sql, None).await,
                };
                open.last_used = Instant::now();
                result
            }
            None => return Err(ExecError::Lost(CoreError::Superseded.to_string())),
        };
        if !cleanup {
            self.leave().await;
        }
        result.map_err(|error| {
            if self.cancel_requested() && !cleanup {
                ExecError::Cancelled
            } else if is_connection_lost(&error) {
                self.broken = true;
                ExecError::Lost(error_text(error))
            } else {
                ExecError::Rejected(error_text(error))
            }
        })
    }

    /// 还回连接。连接断过就丢掉它,下次用时重新连。
    pub async fn release(mut self) {
        self.released = true;
        *self.slot.phase.lock().await = Phase::Idle;
        if self.broken
            && let Some(open) = self.state.take()
        {
            open.conn.close().await;
        }
    }
}

enum Step<'a> {
    Statement(&'a Statement),
    Sql(&'a str),
    Cleanup(&'a str),
}

impl Drop for EditorLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        // 没走 release(出错提前返回、任务被丢弃):尽量复位,别让这个标签页一直显示在执行。
        match self.slot.phase.try_lock() {
            Ok(mut phase) => *phase = Phase::Idle,
            Err(_) => log::warn!("借用的连接没有正常归还"),
        }
        if self.broken {
            self.state.take();
        }
    }
}

/// 驱动流式执行的结果。
pub(crate) struct Driven {
    pub rows: u64,
    pub columns_seen: bool,
    /// 中途停下:`Ok` 是用户取消,`Err` 是写出时出错。
    pub stop: Option<CoreResult<()>>,
    /// 数据库报的错(取消时也会有,由调用方区分)。
    pub error: Option<sqlx::Error>,
}
