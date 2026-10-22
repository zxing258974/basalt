//! 服务器监控:谁连着、在跑什么(会话列表),以及一组只增不减的计数器(查询、事务、流量、缓存读)。
//! 前端隔几秒取一次快照,用前后两次的差算每秒多少;这里只管照实读出来。
//!
//! MySQL / MariaDB 读 `information_schema.PROCESSLIST` 和 `SHOW GLOBAL STATUS`;PostgreSQL 读 `pg_stat_activity`
//! 和 `pg_stat_database`(没有全局的查询计数,只有事务)。SQLite 是嵌在本进程里的,没有服务器可看。
//!
//! 取数和终止都在每个会话自己的一条专用连接上做(第一次用时从池里拿出来、不再放回):
//! 列表里标出的「自己」每次都是同一条,终止也不会恰好落在要终止的那条连接上。

use serde::{Deserialize, Serialize};

use futures_util::future::BoxFuture;

use crate::error::{CoreError, CoreResult};
use crate::session::Session;

/// 一个会话(MySQL 的线程、PostgreSQL 的后端进程)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProcess {
    pub id: i64,
    pub user: Option<String>,
    /// 客户端地址。
    pub host: Option<String>,
    pub database: Option<String>,
    /// MySQL 的 Command(Query、Sleep……),PostgreSQL 的 state(active、idle in transaction……)。
    pub state: Option<String>,
    /// MySQL 的 State(Sending data……),PostgreSQL 在等什么(wait_event_type:wait_event)。
    pub detail: Option<String>,
    /// 处在当前状态多少秒了:在跑的是这条语句跑了多久,空闲的是闲了多久。
    pub seconds: Option<i64>,
    /// 正在执行(或上一条)的语句,最多 [`QUERY_CHARS`] 个字。
    pub query: Option<String>,
    pub activity: Activity,
    /// 会话是什么时候连上的(PostgreSQL 的 backend_start,原样的文本):终止前用它确认还是同一个会话,
    /// 编号可能已经给了新连上的会话。MySQL 的编号只增不减,不需要。
    pub session_start: Option<String>,
    /// 就是 Basalt 取监控数据用的那条连接。
    pub is_self: bool,
}

/// 会话在做什么,决定排序、「隐藏空闲」和能不能终止语句。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    /// 正在执行语句。
    Running,
    /// 没在执行,但开着事务没提交(PostgreSQL 的 idle in transaction),可能占着锁。
    InTransaction,
    Idle,
    /// 服务器自己的线程:事件调度器、复制线程。
    Background,
    /// 账号权限不够,看不到它在做什么。
    Unknown,
}

/// 语句只取前面这么多个字:正在导入的大 INSERT 可能有几 MB,每次刷新都整段传过来不值得。
pub const QUERY_CHARS: usize = 4096;

/// 只增不减的计数器和几个当前值。读不到的(没权限、这种数据库没有)为 `None`。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCounters {
    pub uptime_seconds: Option<u64>,
    /// 当前连接数。
    pub connections: u64,
    pub max_connections: Option<u64>,
    /// 正在执行语句的会话数。
    pub running: u64,
    /// 累计执行的语句(MySQL 的 Questions)。
    pub queries: Option<u64>,
    pub commits: Option<u64>,
    pub rollbacks: Option<u64>,
    pub bytes_received: Option<u64>,
    pub bytes_sent: Option<u64>,
    /// 累计读数据页的次数(含命中缓存的)和其中没命中、要读盘的次数,算缓存命中率用。
    pub cache_reads: Option<u64>,
    pub cache_misses: Option<u64>,
    pub slow_queries: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSnapshot {
    pub counters: ServerCounters,
    pub processes: Vec<ServerProcess>,
    /// 取快照的时刻(Unix 毫秒),前端按它算速率。
    pub taken_at_ms: u64,
    /// 账号权限不够、只看得到一部分会话时的说明。
    pub limited: Option<String>,
}

/// 终止一个会话:只停掉正在执行的语句,还是连同连接一起断开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KillMode {
    Query,
    Connection,
}

/// 要终止的会话,连同用户确认时看到的样子。从确认到执行之间它可能已经换了一条语句、
/// 甚至结束了(PostgreSQL 的进程号还会给新会话用),对不上就不动手。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KillTarget {
    pub id: i64,
    pub mode: KillMode,
    #[serde(default)]
    pub session_start: Option<String>,
    /// 终止语句时:确认时看到的那条语句(快照里给的样子)。
    #[serde(default)]
    pub query: Option<String>,
}

pub(crate) const OWN_CONNECTION: &str = "这是 Basalt 取监控数据用的连接，不能在这里终止。";

pub(crate) fn gone(id: i64) -> CoreError {
    CoreError::Database(format!("会话 {id} 已经结束了。"))
}

pub(crate) fn moved_on(id: i64) -> CoreError {
    CoreError::Database(format!(
        "会话 {id} 已经不在执行刚才那条语句了（执行完了，或者换了一条），没有终止。"
    ))
}

/// 语句截到 [`QUERY_CHARS`] 个字(服务器那边已经截过的话不变)。
pub(crate) fn clip(query: String) -> String {
    match query.char_indices().nth(QUERY_CHARS) {
        Some((end, _)) => query[..end].to_owned(),
        None => query,
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// 监控专用的一条连接和它在服务器上的编号(MySQL 的连接号、PostgreSQL 的进程号)。
pub(crate) struct Dedicated<C> {
    pub conn: C,
    pub id: i64,
}

pub(crate) type DedicatedSlot<C> = tokio::sync::Mutex<Option<Dedicated<C>>>;

/// 在专用连接上做一件事;还没有就用 `open` 开一条。连接断了(网络错误,多半是闲太久被服务器断开)
/// 就丢掉它;用的是放了一阵的旧连接时换一条新的再做一次。别的错误(SQL 报错、会话已经结束)连接照样留着。
///
/// `work` 返回装箱的 `Send` future:Tauri 命令的 future 要能跨线程,
/// 借着参数的异步闭包让编译器证明不了这一点。
pub(crate) async fn with_dedicated<C, T, F>(
    slot: &DedicatedSlot<C>,
    open: impl Fn() -> F,
    work: impl for<'a> Fn(&'a mut Dedicated<C>) -> BoxFuture<'a, CoreResult<T>>,
) -> CoreResult<T>
where
    F: Future<Output = CoreResult<Dedicated<C>>>,
{
    let mut slot = slot.lock().await;
    let mut cached = slot.take();
    // 最多两轮:旧连接断了换一条新的,新的也断了就报出去。
    loop {
        let reused = cached.is_some();
        let mut dedicated = match cached.take() {
            Some(dedicated) => dedicated,
            None => open().await?,
        };
        match work(&mut dedicated).await {
            Err(CoreError::Network(message)) => {
                drop(dedicated);
                if !reused {
                    return Err(CoreError::Network(message));
                }
                log::debug!("监控连接断了，换一条再试：{message}");
            }
            result => {
                *slot = Some(dedicated);
                return result;
            }
        }
    }
}

/// 关会话时先关专用连接(经 SSH 隧道时要在隧道关掉之前)。
pub(crate) async fn close_dedicated<C: sqlx::Connection>(slot: &DedicatedSlot<C>) {
    if let Some(dedicated) = slot.lock().await.take() {
        let _ = dedicated.conn.close().await;
    }
}

const NO_SERVER: &str = "SQLite 是 Basalt 自己打开的文件，没有服务器进程可看。";

impl Session {
    pub async fn server_snapshot(&self) -> CoreResult<ServerSnapshot> {
        match self {
            Self::Mysql(s) => s.server_snapshot().await,
            Self::Postgres(s) => s.server_snapshot().await,
            Self::Sqlite(_) => Err(CoreError::Unsupported(NO_SERVER.into())),
        }
    }

    pub async fn kill_process(&self, target: &KillTarget) -> CoreResult<()> {
        match self {
            Self::Mysql(s) => s.kill_process(target).await,
            Self::Postgres(s) => s.kill_process(target).await,
            Self::Sqlite(_) => Err(CoreError::Unsupported(NO_SERVER.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicI64, Ordering};

    /// 假连接:编号就是第几次打开的。
    async fn open(opened: &AtomicI64) -> CoreResult<Dedicated<()>> {
        let id = opened.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Dedicated { conn: (), id })
    }

    fn fails_on(
        id: i64,
        error: fn() -> CoreError,
    ) -> impl Fn(&mut Dedicated<()>) -> BoxFuture<'_, CoreResult<i64>> {
        move |d| {
            let result = if d.id == id { Err(error()) } else { Ok(d.id) };
            Box::pin(async move { result })
        }
    }

    fn network() -> CoreError {
        CoreError::Network("broken pipe".into())
    }

    #[tokio::test]
    async fn the_dedicated_connection_is_opened_once_and_kept() {
        let slot = DedicatedSlot::default();
        let opened = AtomicI64::new(0);
        for _ in 0..3 {
            let id = with_dedicated(&slot, || open(&opened), fails_on(0, network)).await;
            assert_eq!(id.unwrap(), 1);
        }
        assert_eq!(opened.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_dropped_connection_is_replaced_and_the_work_retried_once() {
        let slot = DedicatedSlot::default();
        let opened = AtomicI64::new(0);
        with_dedicated(&slot, || open(&opened), fails_on(0, network))
            .await
            .unwrap();
        let id = with_dedicated(&slot, || open(&opened), fails_on(1, network)).await;
        assert_eq!(id.unwrap(), 2);
        assert_eq!(slot.lock().await.as_ref().map(|d| d.id), Some(2));

        // 新开的也断了:报出去,不留着。
        let error = with_dedicated(
            &slot,
            || open(&opened),
            |_| Box::pin(async { Err::<i64, _>(network()) }),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, CoreError::Network(_)));
        assert_eq!(opened.load(Ordering::SeqCst), 3);
        assert!(slot.lock().await.is_none());
    }

    #[tokio::test]
    async fn other_errors_keep_the_connection_and_are_not_retried() {
        let slot = DedicatedSlot::default();
        let opened = AtomicI64::new(0);
        let error = with_dedicated(&slot, || open(&opened), fails_on(1, || gone(7)))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("已经结束"));
        assert_eq!(opened.load(Ordering::SeqCst), 1);
        assert!(slot.lock().await.is_some());
    }

    #[test]
    fn long_statements_are_clipped_on_a_character_boundary() {
        let short = "SELECT 1".to_owned();
        assert_eq!(clip(short.clone()), short);
        let long = "中".repeat(QUERY_CHARS + 10);
        let clipped = clip(long);
        assert_eq!(clipped.chars().count(), QUERY_CHARS);
        assert!(clipped.chars().all(|c| c == '中'));
    }
}
