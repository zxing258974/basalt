//! MySQL / MariaDB 的服务器监控:`information_schema.PROCESSLIST`、`SHOW GLOBAL STATUS`,终止用 `KILL [QUERY]`。
//! 都在会话的监控专用连接上做,见 [`crate::monitor`]。

use std::collections::HashMap;

use sqlx::mysql::{MySqlConnection, MySqlRow};
use sqlx::{Executor, Row};

use super::mysql::MysqlSession;
use crate::error::{CoreError, CoreResult};
use crate::monitor::{
    Activity, Dedicated, KillMode, KillTarget, OWN_CONNECTION, QUERY_CHARS, ServerCounters,
    ServerProcess, ServerSnapshot, clip, gone, moved_on, now_ms, with_dedicated,
};
use crate::sql::dynamic;

/// 列的顺序和 `SHOW FULL PROCESSLIST` 一样;语句在服务器上就截短(`?` 是 [`QUERY_CHARS`])。
/// MySQL 8 说这张表以后会去掉,读不了时退回 `SHOW FULL PROCESSLIST`。
const PROCESSLIST: &str = "SELECT ID, USER, HOST, DB, COMMAND, TIME, STATE, LEFT(INFO, ?) \
    FROM information_schema.PROCESSLIST";

const STATUS: &str = "SHOW GLOBAL STATUS WHERE Variable_name IN (
    'Uptime', 'Threads_connected', 'Threads_running', 'Questions', 'Com_commit', 'Com_rollback',
    'Bytes_received', 'Bytes_sent', 'Innodb_buffer_pool_read_requests', 'Innodb_buffer_pool_reads',
    'Slow_queries')";

const LIMITED: &str = "这个账号没有 PROCESS 权限，只看得到它自己的会话。";

/// 服务器自己的线程:事件调度器、复制(主库给从库发 binlog 的、从库收和重放的)。
const BACKGROUND_COMMANDS: [&str; 8] = [
    "Daemon",
    "Binlog Dump",
    "Binlog Dump GTID",
    "Register Slave",
    "Register Replica",
    "Slave_IO",
    "Slave_SQL",
    "Slave_worker",
];
/// MySQL 8 的复制线程以 `system user` 出现,命令是普通的 Connect / Query。
const BACKGROUND_USERS: [&str; 2] = ["system user", "event_scheduler"];

impl MysqlSession {
    async fn open_monitor(&self) -> CoreResult<Dedicated<MySqlConnection>> {
        let mut conn = self.pool.acquire().await?.detach();
        let row = (&mut conn)
            .fetch_one("SELECT CAST(CONNECTION_ID() AS UNSIGNED)")
            .await?;
        let id = int(&row, 0).unwrap_or_default();
        Ok(Dedicated { conn, id })
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

async fn snapshot(dedicated: &mut Dedicated<MySqlConnection>) -> CoreResult<ServerSnapshot> {
    let own = dedicated.id;
    let conn = &mut dedicated.conn;
    let processes = read_processes(conn, own).await?;
    let status: HashMap<String, u64> = (&mut *conn)
        .fetch_all(STATUS)
        .await?
        .iter()
        .filter_map(|row| {
            let name = text(row, 0)?;
            let value = text(row, 1)?.trim().parse().ok()?;
            Some((name.to_ascii_lowercase(), value))
        })
        .collect();
    // 速率按计数器读出来的时刻算,后面查权限的往返不算进去。
    let taken_at_ms = now_ms();
    let max_connections = int(
        &(&mut *conn).fetch_one("SELECT @@max_connections").await?,
        0,
    )
    .and_then(|n| u64::try_from(n).ok());
    let limited = match (&mut *conn).fetch_all("SHOW GRANTS").await {
        Ok(rows) => {
            let grants: Vec<String> = rows.iter().filter_map(|row| text(row, 0)).collect();
            (sees_all_threads(&grants) == Some(false)).then(|| LIMITED.to_owned())
        }
        Err(error) => {
            log::debug!("读不了授权：{error}");
            None
        }
    };
    let get = |name: &str| status.get(name).copied();
    Ok(ServerSnapshot {
        counters: ServerCounters {
            uptime_seconds: get("uptime"),
            connections: get("threads_connected").unwrap_or_default(),
            max_connections,
            running: get("threads_running").unwrap_or_default(),
            queries: get("questions"),
            commits: get("com_commit"),
            rollbacks: get("com_rollback"),
            bytes_received: get("bytes_received"),
            bytes_sent: get("bytes_sent"),
            cache_reads: get("innodb_buffer_pool_read_requests"),
            cache_misses: get("innodb_buffer_pool_reads"),
            slow_queries: get("slow_queries"),
        },
        processes,
        taken_at_ms,
        limited,
    })
}

/// 先确认会话还在、终止语句时还在执行确认时看到的那条,再 KILL。
/// 连接号只增不减,不会给了别的会话;两步之间的那一瞬间换了语句的话拦不住。
async fn kill(dedicated: &mut Dedicated<MySqlConnection>, target: KillTarget) -> CoreResult<()> {
    let id = target.id;
    if id <= 0 {
        return Err(CoreError::InvalidData(format!("会话编号不对：{id}")));
    }
    if id == dedicated.id {
        return Err(CoreError::InvalidData(OWN_CONNECTION.into()));
    }
    let current = read_processes(&mut dedicated.conn, dedicated.id)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| gone(id))?;
    // 编号是整数,直接拼;KILL 的参数不能用占位符。
    let sql = match target.mode {
        KillMode::Query => {
            let same = target.query.is_none() || current.query == target.query;
            if current.activity != Activity::Running || !same {
                return Err(moved_on(id));
            }
            format!("KILL QUERY {id}")
        }
        KillMode::Connection => format!("KILL {id}"),
    };
    (&mut dedicated.conn).execute(dynamic(sql)).await?;
    Ok(())
}

async fn read_processes(conn: &mut MySqlConnection, own: i64) -> CoreResult<Vec<ServerProcess>> {
    let limit = i64::try_from(QUERY_CHARS).unwrap_or(i64::MAX);
    let rows = match sqlx::query(PROCESSLIST)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            log::debug!(
                "information_schema.PROCESSLIST 读不了，改用 SHOW FULL PROCESSLIST：{error}"
            );
            conn.fetch_all("SHOW FULL PROCESSLIST").await?
        }
    };
    Ok(rows.iter().map(|row| process(row, own)).collect())
}

fn process(row: &MySqlRow, own: i64) -> ServerProcess {
    let id = int(row, 0).unwrap_or_default();
    let user = text(row, 1);
    let state = text(row, 4);
    ServerProcess {
        id,
        activity: activity(state.as_deref(), user.as_deref()),
        user,
        host: text(row, 2),
        database: text(row, 3),
        state,
        seconds: int(row, 5),
        detail: text(row, 6).filter(|s| !s.is_empty()),
        query: text(row, 7).map(clip),
        session_start: None,
        is_self: id == own,
    }
}

fn activity(command: Option<&str>, user: Option<&str>) -> Activity {
    match command {
        None => Activity::Unknown,
        Some("Sleep") => Activity::Idle,
        Some(command)
            if BACKGROUND_COMMANDS.contains(&command)
                || user.is_some_and(|u| BACKGROUND_USERS.contains(&u)) =>
        {
            Activity::Background
        }
        Some(_) => Activity::Running,
    }
}

/// 看得到所有人的会话要全局的 PROCESS 权限(ALL 里含着它;SUPER 不算)。
/// 授权里有角色时 `SHOW GRANTS` 不展开角色的权限,说不准,返回 None。
fn sees_all_threads(grants: &[String]) -> Option<bool> {
    let mut sees_all = false;
    for grant in grants {
        let upper = grant.to_ascii_uppercase();
        let Some(rest) = upper.strip_prefix("GRANT ") else {
            continue;
        };
        // 授予角色的那一行没有 ON:「GRANT `role`@`%` TO …」。
        let on = rest.find(" ON ")?;
        let target = rest[on + 4..].trim_start();
        if !target.starts_with("*.*") {
            continue;
        }
        sees_all |= rest[..on]
            .split(',')
            .map(str::trim)
            .any(|privilege| matches!(privilege, "PROCESS" | "ALL" | "ALL PRIVILEGES"));
    }
    Some(sees_all)
}

/// 文本列:二进制排序规则的列 MySQL 以字节给。
fn text(row: &MySqlRow, index: usize) -> Option<String> {
    match row.try_get::<Option<String>, _>(index) {
        Ok(value) => value,
        Err(_) => row
            .try_get::<Option<Vec<u8>>, _>(index)
            .ok()
            .flatten()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
    }
}

/// 整数列:各版本的类型不一样(有符号、无符号、甚至文本),都试一遍。
fn int(row: &MySqlRow, index: usize) -> Option<i64> {
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value;
    }
    if let Ok(value) = row.try_get::<Option<u64>, _>(index) {
        return value.and_then(|v| i64::try_from(v).ok());
    }
    if let Ok(value) = row.try_get::<Option<i32>, _>(index) {
        return value.map(i64::from);
    }
    text(row, index)?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grants(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn only_a_global_process_or_all_grant_sees_every_thread() {
        let root = grants(&["GRANT ALL PRIVILEGES ON *.* TO 'root'@'%' WITH GRANT OPTION"]);
        assert_eq!(sees_all_threads(&root), Some(true));
        let monitor = grants(&["GRANT SELECT, PROCESS ON *.* TO `mon`@`%`"]);
        assert_eq!(sees_all_threads(&monitor), Some(true));
        let app = grants(&[
            "GRANT USAGE ON *.* TO `app`@`%`",
            "GRANT ALL PRIVILEGES ON `shop`.* TO `app`@`%`",
        ]);
        assert_eq!(sees_all_threads(&app), Some(false));
    }

    #[test]
    fn names_and_super_do_not_count_as_process() {
        let named = grants(&["GRANT USAGE ON *.* TO `process_exporter`@`%`"]);
        assert_eq!(sees_all_threads(&named), Some(false));
        let superuser = grants(&["GRANT SUPER ON *.* TO `supervisor`@`%`"]);
        assert_eq!(sees_all_threads(&superuser), Some(false));
    }

    #[test]
    fn role_grants_leave_it_unknown() {
        let mysql = grants(&[
            "GRANT USAGE ON *.* TO `u`@`%`",
            "GRANT `ops`@`%` TO `u`@`%`",
        ]);
        assert_eq!(sees_all_threads(&mysql), None);
        let mariadb = grants(&["GRANT `ops` TO `u`@`%`", "GRANT USAGE ON *.* TO `u`@`%`"]);
        assert_eq!(sees_all_threads(&mariadb), None);
    }

    #[test]
    fn server_threads_are_background() {
        assert_eq!(activity(Some("Sleep"), Some("app")), Activity::Idle);
        assert_eq!(activity(Some("Query"), Some("app")), Activity::Running);
        assert_eq!(
            activity(Some("Daemon"), Some("event_scheduler")),
            Activity::Background
        );
        assert_eq!(
            activity(Some("Binlog Dump GTID"), Some("repl")),
            Activity::Background
        );
        assert_eq!(
            activity(Some("Connect"), Some("system user")),
            Activity::Background
        );
        assert_eq!(activity(None, None), Activity::Unknown);
    }
}
