//! 每个查询标签页一条专用连接,按标签页 id 管理:第一次执行时才建立,
//! 标签页关闭或所属连接关闭时断开。同一个标签页的语句串行执行。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use super::collect::{Collected, CollectedSet};
use super::results::{FIRST_BATCH, RELEASED, ResultSort, ResultStore};
use super::{
    OutcomeStatus, QueryScope, ResultColumn, ResultSet, StatementOutcome, is_ddl, is_dml,
    leading_keyword, may_change_scope, may_return_rows,
};
use crate::driver::editor::{CancelHandle, EditorConnection, Sources};
use crate::error::{CoreError, CoreResult};
use crate::session::Session;
use crate::value::CellValue;

/// 关闭标签页时最多等正在执行的语句收尾多久;等不到就不等了,连接随最后一个引用一起断开。
const CLOSE_WAIT: Duration = Duration::from_secs(3);
/// 闲置超过这么久才在执行前 ping 一下:连续执行一批语句时不必每条都多一次往返。
const PING_AFTER_IDLE: Duration = Duration::from_secs(30);

const COPY_UNSUPPORTED: &str = "COPY … FROM STDIN / TO STDOUT 要客户端收发数据，SQL 编辑器不支持；这条连接已经重置。导入导出数据请用 INSERT，或等之后的导入导出功能。";

const RECONNECTED: &str =
    "与服务器的连接断开过，已经重新连上；之前会话里的变量、临时表和未提交的事务都没有了。";

/// 已经建立的专用连接。`session` 记住它是从哪一次打开的会话里来的:连接关掉再打开以后,旧连接作废。
pub(super) struct OpenEditor {
    session: Weak<Session>,
    pub(super) conn: EditorConnection,
    pub(super) scope: QueryScope,
    pub(super) last_used: Instant,
}

pub(super) struct Running {
    pub(super) handle: CancelHandle,
    pub(super) requested: Arc<AtomicBool>,
}

/// 标签页上这一次执行走到哪了。还在准备(建连接、切库)时取消,就不再开始执行。
pub(super) enum Phase {
    Idle,
    Preparing {
        cancel_requested: bool,
    },
    Running(Running),
    /// 导入导出借走了这条连接(见 [`super::lease`]),两条语句之间:取消只记下来。
    Leased(Arc<AtomicBool>),
}

pub(super) struct EditorSlot {
    connection_id: String,
    /// 导入导出借用时要跨方法持有这把锁,所以放在 Arc 里(`lock_owned`)。
    pub(super) state: Arc<Mutex<Option<OpenEditor>>>,
    pub(super) phase: Mutex<Phase>,
    /// 执行出来的结果。不放在 `state` 里:执行下一条语句时也要能往下翻上一条的结果。
    results: std::sync::Mutex<ResultStore>,
}

impl EditorSlot {
    fn new(connection_id: &str) -> Self {
        Self {
            connection_id: connection_id.to_owned(),
            state: Arc::new(Mutex::new(None)),
            phase: Mutex::new(Phase::Idle),
            results: std::sync::Mutex::new(ResultStore::default()),
        }
    }

    fn results(&self) -> std::sync::MutexGuard<'_, ResultStore> {
        self.results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 执行中的发取消请求,准备中的记下来。返回是不是真有东西可取消。
    /// 持锁期间说完取消:语句收尾要等这把锁,KILL 不会落到下一条语句上。
    async fn request_cancel(&self) -> CoreResult<bool> {
        let mut phase = self.phase.lock().await;
        match &mut *phase {
            Phase::Idle => Ok(false),
            Phase::Preparing { cancel_requested } => {
                *cancel_requested = true;
                Ok(true)
            }
            Phase::Running(running) => {
                running.requested.store(true, Ordering::Relaxed);
                running.handle.cancel().await?;
                Ok(true)
            }
            Phase::Leased(requested) => {
                requested.store(true, Ordering::Relaxed);
                Ok(true)
            }
        }
    }

    /// 取消还在执行的语句,再断开连接。
    async fn shut_down(&self) {
        if let Err(error) = self.request_cancel().await {
            log::warn!("关闭查询标签页时取消语句失败：{error}");
        }
        match tokio::time::timeout(CLOSE_WAIT, self.state.lock()).await {
            Ok(mut state) => {
                if let Some(open) = state.take() {
                    open.conn.close().await;
                }
            }
            Err(_) => log::warn!("查询标签页的语句迟迟不结束，不再等它，连接会在执行结束后断开"),
        }
    }
}

#[derive(Default)]
pub struct EditorRegistry {
    slots: std::sync::Mutex<HashMap<String, Arc<EditorSlot>>>,
}

impl EditorRegistry {
    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<EditorSlot>>> {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 标签页换了连接时,旧连接上的那条专用连接一并断开。
    pub(super) async fn slot(&self, editor_id: &str, connection_id: &str) -> Arc<EditorSlot> {
        let (slot, replaced) = {
            let mut slots = self.slots();
            match slots.get(editor_id) {
                Some(slot) if slot.connection_id == connection_id => (slot.clone(), None),
                _ => {
                    let slot = Arc::new(EditorSlot::new(connection_id));
                    let replaced = slots.insert(editor_id.to_owned(), slot.clone());
                    (slot, replaced)
                }
            }
        };
        if let Some(old) = replaced {
            old.shut_down().await;
        }
        slot
    }

    /// 在标签页的专用连接上执行一条语句。
    pub async fn run(
        &self,
        editor_id: &str,
        connection_id: &str,
        session: &Arc<Session>,
        scope: &QueryScope,
        sql: &str,
    ) -> CoreResult<StatementOutcome> {
        let slot = self.slot(editor_id, connection_id).await;
        let mut state = slot.state.lock().await;
        *slot.phase.lock().await = Phase::Preparing {
            cancel_requested: false,
        };
        let result = execute(&slot, &mut state, session, scope, sql).await;
        *slot.phase.lock().await = Phase::Idle;
        result
    }

    /// 取消标签页正在执行(或正在准备执行)的语句。什么都没在做时返回 false。
    pub async fn cancel(&self, editor_id: &str) -> CoreResult<bool> {
        let slot = self.slots().get(editor_id).cloned();
        match slot {
            Some(slot) => slot.request_cancel().await,
            None => Ok(false),
        }
    }

    /// 取一个结果集里的一段行,给了排序就按排序后的顺序。
    pub async fn result_rows(
        &self,
        editor_id: &str,
        result_id: u64,
        offset: usize,
        limit: usize,
        sort: Option<ResultSort>,
    ) -> CoreResult<Vec<Vec<CellValue>>> {
        let slot = self.slots().get(editor_id).cloned();
        let stored = match slot {
            Some(slot) => slot.results().get(result_id)?,
            None => return Err(CoreError::NotFound(RELEASED.into())),
        };
        stored.rows(offset, limit, sort).await
    }

    /// 前端不再显示这些结果了(重新执行、关了结果),放掉它们占的内存。
    pub fn release_results(&self, editor_id: &str, result_ids: &[u64]) {
        let slot = self.slots().get(editor_id).cloned();
        if let Some(slot) = slot {
            // 先放开锁再释放:大结果释放要一会儿,别让同一个标签页的取行等着。
            let released = slot.results().release(result_ids);
            drop(released);
        }
    }

    /// 标签页关了:断开它的专用连接。
    pub async fn close(&self, editor_id: &str) {
        let slot = self.slots().remove(editor_id);
        if let Some(slot) = slot {
            slot.shut_down().await;
        }
    }

    /// 连接关了:断开它名下所有标签页的专用连接。要在关连接池之前调用,取消请求还要用池。
    pub async fn close_connection(&self, connection_id: &str) {
        let removed: Vec<Arc<EditorSlot>> = {
            let mut slots = self.slots();
            let ids: Vec<String> = slots
                .iter()
                .filter(|(_, slot)| slot.connection_id == connection_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| slots.remove(id)).collect()
        };
        for slot in removed {
            slot.shut_down().await;
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slots().len()
    }
}

/// 准备好连接后执行;准备期间被取消就不执行了。
async fn execute(
    slot: &EditorSlot,
    state: &mut Option<OpenEditor>,
    session: &Arc<Session>,
    scope: &QueryScope,
    sql: &str,
) -> CoreResult<StatementOutcome> {
    let notice = prepare(state, session, scope).await?;
    // prepare 成功时一定留下了一条连接。
    let Some(open) = state.as_mut() else {
        return Err(CoreError::Superseded);
    };
    let handle = open.conn.cancel_handle(session).await?;
    let requested = Arc::new(AtomicBool::new(false));
    {
        let mut phase = slot.phase.lock().await;
        if matches!(
            *phase,
            Phase::Preparing {
                cancel_requested: true
            }
        ) {
            return Ok(cancelled_before_start(open.scope.clone(), notice));
        }
        *phase = Phase::Running(Running {
            handle: handle.clone(),
            requested: requested.clone(),
        });
    }
    let started = Instant::now();
    let collected = open.conn.run(sql, &handle).await;
    let elapsed = started.elapsed();
    open.last_used = Instant::now();
    // 取消请求进行到一半时要等它说完(它持着这把锁),免得 KILL 落到下一条语句上。
    *slot.phase.lock().await = Phase::Idle;
    let cancelled = requested.load(Ordering::Relaxed);
    let broken = collected.error.as_ref().is_some_and(is_connection_lost);
    let mut outcome = finish(open, slot, sql, collected, cancelled, elapsed).await;
    outcome.notice = notice;
    if broken {
        // 连接已经断了:丢掉它,下一次执行时重新连。
        if let Some(open) = state.take() {
            open.conn.close().await;
        }
    }
    Ok(outcome)
}

fn cancelled_before_start(scope: QueryScope, notice: Option<String>) -> StatementOutcome {
    StatementOutcome {
        status: OutcomeStatus::Cancelled,
        message: Some("还没开始执行就取消了".into()),
        result_sets: Vec::new(),
        affected_rows: None,
        elapsed_ms: 0,
        scope,
        notice,
    }
}

/// 确保有一条可用的专用连接,并切到要求的库 / schema。返回要告诉用户的事。
pub(super) async fn prepare(
    state: &mut Option<OpenEditor>,
    session: &Arc<Session>,
    scope: &QueryScope,
) -> CoreResult<Option<String>> {
    let current = Arc::downgrade(session);
    let mut notice = None;
    if let Some(open) = state.as_mut() {
        let stale = !Weak::ptr_eq(&open.session, &current);
        let other_database = !open.conn.can_switch_to(scope);
        let reconnect = stale || other_database;
        let idle = open.last_used.elapsed() > PING_AFTER_IDLE;
        if !reconnect && idle && !open.conn.ping().await {
            notice = Some(RECONNECTED.to_owned());
        }
        if other_database && !stale {
            notice = Some(format!(
                "PostgreSQL 换到库「{}」要新开一条连接：之前会话里的设置、临时表和未提交的事务不会带过去。",
                scope.database.as_deref().unwrap_or_default()
            ));
        }
        if (reconnect || notice.is_some())
            && let Some(old) = state.take()
        {
            old.conn.close().await;
        }
    }
    match state.as_mut() {
        Some(open) => open.scope = open.conn.switch(&open.scope, scope).await?,
        None => {
            let (conn, actual) = EditorConnection::open(session, scope).await?;
            *state = Some(OpenEditor {
                session: current,
                conn,
                scope: actual,
                last_used: Instant::now(),
            });
        }
    }
    Ok(notice)
}

/// 连接层面的失败(断网、服务器关了连接):这条连接不能再用。
pub(super) fn is_connection_lost(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Io(_) | sqlx::Error::Protocol(_) | sqlx::Error::WorkerCrashed
    )
}

/// 数据库报错带上错误号(MySQL)或详情、提示(PostgreSQL),和命令行客户端看到的一样。
pub(super) fn error_text(error: sqlx::Error) -> String {
    let sqlx::Error::Database(db) = &error else {
        return CoreError::from(error).to_string();
    };
    if let Some(mysql) = db.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() {
        return format!("{} - {}", mysql.number(), mysql.message());
    }
    if let Some(pg) = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>() {
        let mut text = pg.message().to_owned();
        if let Some(detail) = pg.detail() {
            text.push_str(&format!("\n详情：{detail}"));
        }
        if let Some(hint) = pg.hint() {
            text.push_str(&format!("\n提示：{hint}"));
        }
        return text;
    }
    db.message().to_owned()
}

/// 把读到的东西整理成给前端的结果:补空结果的列名、算影响行数、刷新所在的库、存下结果。
async fn finish(
    open: &mut OpenEditor,
    slot: &EditorSlot,
    sql: &str,
    collected: Collected,
    cancelled: bool,
    elapsed: Duration,
) -> StatementOutcome {
    let keyword = leading_keyword(sql);
    let Collected {
        mut result_sets,
        affected,
        error,
    } = collected;
    if error.is_none() && result_sets.is_empty() && may_return_rows(&keyword) {
        result_sets.extend(open.conn.describe(sql).await);
    }
    let trusted = !open.conn.changes_only_for_dml() || is_dml(&keyword);
    let affected_rows = (result_sets.is_empty() && trusted && (is_dml(&keyword) || affected > 0))
        .then_some(affected);
    if may_change_scope(&keyword) {
        open.scope = open.conn.current_scope(&open.scope).await;
    }
    if is_ddl(&keyword) {
        // 表结构变了:sqlx 按 SQL 文本缓存的预备语句(取空结果列名用的)会带着旧列信息。
        open.conn.clear_cached_statements().await;
    }
    let result_sets = keep(open, slot, result_sets, error.is_none()).await;
    let (status, message) = match error {
        None => (OutcomeStatus::Ok, None),
        Some(error) => (
            if cancelled {
                OutcomeStatus::Cancelled
            } else {
                OutcomeStatus::Error
            },
            Some(
                if keyword == "COPY" && matches!(error, sqlx::Error::Protocol(_)) {
                    COPY_UNSUPPORTED.to_owned()
                } else {
                    error_text(error)
                },
            ),
        ),
    };
    StatementOutcome {
        status,
        message,
        result_sets,
        affected_rows,
        elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        scope: open.scope.clone(),
        notice: None,
    }
}

/// 结果存进标签页的结果存储,只把第一批行带给前端。语句顺利执行完时才去查列的来源
/// (MySQL 要在这条连接上确认是不是临时表、PostgreSQL 要查一次系统目录;出错、取消后不再多发语句)。
async fn keep(
    open: &mut OpenEditor,
    slot: &EditorSlot,
    sets: Vec<CollectedSet>,
    with_sources: bool,
) -> Vec<ResultSet> {
    let mut kept = Vec::with_capacity(sets.len());
    let session = open.session.upgrade();
    for set in sets {
        let sources = if with_sources {
            open.conn
                .resolve_sources(session.as_deref(), &open.scope, &set.origins)
                .await
        } else {
            Sources {
                columns: vec![None; set.columns.len()],
                read_only: None,
            }
        };
        let columns: Vec<ResultColumn> = set
            .columns
            .into_iter()
            .zip(sources.columns)
            .map(|(column, source)| ResultColumn { source, ..column })
            .collect();
        let rows = set.rows[..set.rows.len().min(FIRST_BATCH)].to_vec();
        let total_rows = set.rows.len() as u64;
        let (id, evicted) = slot.results().add(columns.len(), set.rows, set.bytes);
        drop(evicted);
        kept.push(ResultSet {
            id,
            columns,
            rows,
            total_rows,
            truncated: set.truncated,
            read_only: sources.read_only,
        });
    }
    kept
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
