//! PostgreSQL 的服务器监控:`pg_stat_activity` 里的客户端会话、`pg_stat_database` 的累计数,
//! 终止用 `pg_cancel_backend` / `pg_terminate_backend`。没有全局的查询计数,只有事务。
//! 都在会话的监控专用连接上做,见 [`crate::monitor`]。

use sqlx::Row;
use sqlx::postgres::{PgConnection, PgRow};

use super::postgres::PgSession;
use crate::error::{CoreError, CoreResult};
use crate::monitor::{
    Activity, Dedicated, KillMode, KillTarget, OWN_CONNECTION, QUERY_CHARS, ServerCounters,
    ServerProcess, ServerSnapshot, gone, moved_on, now_ms, with_dedicated,
};

/// 客户端连上来的会话。没有权限看的会话 backend_type 也是空的,靠「连着库、有用户」认出来
/// (后台进程不连库或者没有用户)。
macro_rules! client_sessions {
    () => {
        "(backend_type = 'client backend' \
         OR (backend_type IS NULL AND datid IS NOT NULL AND usesysid IS NOT NULL))"
    };
}

/// 空闲的按闲了多久算,在跑的按这条语句跑了多久算;没权限看的会话这些都是空的。
/// 语句截到 `$1`([`QUERY_CHARS`])个字;没权限时 PostgreSQL 给的是一句提示,不当语句。
const ACTIVITY: &str = concat!(
    "SELECT pid::int8, usename::text, \
        CASE WHEN client_addr IS NULL THEN NULL ELSE host(client_addr) || ':' || client_port END, \
        datname::text, state, \
        CASE WHEN wait_event IS NULL THEN NULL ELSE wait_event_type || ':' || wait_event END, \
        EXTRACT(EPOCH FROM now() - COALESCE( \
            CASE WHEN state = 'active' THEN query_start END, state_change, backend_start))::int8, \
        CASE WHEN backend_type IS NULL THEN NULL ELSE left(query, $1) END, \
        backend_start::text, pid = pg_backend_pid() \
    FROM pg_stat_activity WHERE ",
    client_sessions!(),
    " ORDER BY backend_start"
);

const COUNTERS: &str = concat!(
    "SELECT \
        (SELECT count(*) FROM pg_stat_activity WHERE ",
    client_sessions!(),
    ")::int8, \
        (SELECT count(*) FROM pg_stat_activity \
            WHERE backend_type = 'client backend' AND state = 'active')::int8, \
        current_setting('max_connections')::int8, \
        EXTRACT(EPOCH FROM now() - pg_postmaster_start_time())::int8, \
        sum(xact_commit)::int8, sum(xact_rollback)::int8, \
        sum(blks_hit + blks_read)::int8, sum(blks_read)::int8 \
    FROM pg_stat_database"
);

/// 能看到别人在做什么要超级用户,或者能用 pg_read_all_stats 的权限(继承来的才算,所以是 USAGE 不是 MEMBER)。
const VISIBILITY: &str = "SELECT rolsuper OR pg_has_role(current_user, 'pg_read_all_stats', 'USAGE') \
    FROM pg_roles WHERE rolname = current_user";

const LIMITED: &str =
    "这个账号不是超级用户、也没有 pg_read_all_stats 权限：看得到别人的会话，看不到他们在做什么。";

/// 终止前再看一眼这个进程号现在是谁、在做什么。
const CURRENT: &str = "SELECT backend_start::text, state, left(query, $2) \
    FROM pg_stat_activity WHERE pid = $1";

impl PgSession {
    async fn open_monitor(&self) -> CoreResult<Dedicated<PgConnection>> {
        let pool = self.pool(self.default_database()).await?;
        let mut conn = pool.acquire().await?.detach();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut conn)
            .await?;
        Ok(Dedicated {
            conn,
            id: i64::from(pid),
        })
    }

    pub(crate) async fn server_snapshot(&self) -> CoreResult<ServerSnapshot> {
        with_dedicated(
            &self.monitor,
            || self.open_monitor(),
            |d| Box::pin(snapshot(d)),
        )
        .await
    }

    pub(crate) async fn kill_process(&self, target: &KillTarget) -> CoreResult<()> {
        with_dedicated(
            &self.monitor,
            || self.open_monitor(),
            |d| Box::pin(kill(d, target.clone())),
        )
        .await
    }
}

fn query_chars() -> i32 {
    i32::try_from(QUERY_CHARS).unwrap_or(i32::MAX)
}

async fn snapshot(dedicated: &mut Dedicated<PgConnection>) -> CoreResult<ServerSnapshot> {
    let conn = &mut dedicated.conn;
    let processes = sqlx::query(ACTIVITY)
        .bind(query_chars())
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(process)
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let row = sqlx::query(COUNTERS).fetch_one(&mut *conn).await?;
    // 速率按计数器读出来的时刻算,后面查权限的往返不算进去。
    let taken_at_ms = now_ms();
    let count = |index: usize| -> Result<Option<u64>, sqlx::Error> {
        Ok(row
            .try_get::<Option<i64>, _>(index)?
            .and_then(|n| u64::try_from(n).ok()))
    };
    let counters = ServerCounters {
        connections: count(0)?.unwrap_or_default(),
        running: count(1)?.unwrap_or_default(),
        max_connections: count(2)?,
        uptime_seconds: count(3)?,
        queries: None,
        commits: count(4)?,
        rollbacks: count(5)?,
        bytes_received: None,
        bytes_sent: None,
        cache_reads: count(6)?,
        cache_misses: count(7)?,
        slow_queries: None,
    };
    let sees_all: Option<bool> = sqlx::query_scalar(VISIBILITY)
        .fetch_optional(&mut *conn)
        .await
        .unwrap_or_else(|error| {
            log::debug!("读不了角色：{error}");
            Some(true)
        });
    Ok(ServerSnapshot {
        counters,
        processes,
        taken_at_ms,
        limited: (sees_all == Some(false)).then(|| LIMITED.to_owned()),
    })
}

/// 先确认这个进程号还是确认时那个会话(按连上的时刻)、终止语句时还在执行那条语句,再发信号。
/// 两步之间的那一瞬间换了语句的话拦不住。
async fn kill(dedicated: &mut Dedicated<PgConnection>, target: KillTarget) -> CoreResult<()> {
    let id = target.id;
    let pid =
        i32::try_from(id).map_err(|_| CoreError::InvalidData(format!("会话编号不对：{id}")))?;
    if id == dedicated.id {
        return Err(CoreError::InvalidData(OWN_CONNECTION.into()));
    }
    let conn = &mut dedicated.conn;
    let current: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(CURRENT)
        .bind(pid)
        .bind(query_chars())
        .fetch_optional(&mut *conn)
        .await?;
    let Some((started, state, query)) = current else {
        return Err(gone(id));
    };
    if target.session_start.is_some() && started != target.session_start {
        return Err(gone(id));
    }
    let sql = match target.mode {
        KillMode::Query => {
            let same = target.query.is_none() || query == target.query;
            if state.as_deref() != Some("active") || !same {
                return Err(moved_on(id));
            }
            "SELECT pg_cancel_backend($1)"
        }
        KillMode::Connection => "SELECT pg_terminate_backend($1)",
    };
    let done: bool = sqlx::query_scalar(sql)
        .bind(pid)
        .fetch_one(&mut *conn)
        .await?;
    if done {
        Ok(())
    } else {
        Err(CoreError::Database(format!(
            "没能终止会话 {pid}：它已经结束了，或者不是这个账号的会话（要超级用户或 pg_signal_backend 权限）"
        )))
    }
}

fn process(row: &PgRow) -> Result<ServerProcess, sqlx::Error> {
    let state: Option<String> = row.try_get(4)?;
    Ok(ServerProcess {
        id: row.try_get(0)?,
        user: row.try_get(1)?,
        host: row.try_get(2)?,
        database: row.try_get(3)?,
        activity: activity(state.as_deref()),
        state,
        detail: row.try_get(5)?,
        seconds: row.try_get(6)?,
        query: row
            .try_get::<Option<String>, _>(7)?
            .filter(|q| !q.is_empty()),
        session_start: row.try_get(8)?,
        is_self: row.try_get(9)?,
    })
}

fn activity(state: Option<&str>) -> Activity {
    match state {
        Some("active" | "fastpath function call") => Activity::Running,
        Some("idle in transaction" | "idle in transaction (aborted)") => Activity::InTransaction,
        Some("idle") => Activity::Idle,
        // 没权限看(空),或者服务器关了 track_activities(disabled)。
        _ => Activity::Unknown,
    }
}
