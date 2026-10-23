//! Redis 的服务器监控:`INFO` 里的计数器和内存、`CLIENT LIST` 的客户端、`SLOWLOG` 的慢命令,
//! 装进和 SQL 数据库一样的快照,界面照用同一套。
//! 终止:断开连接用 `CLIENT KILL ID`;「停下正在执行的」对 Redis 是解除阻塞(`CLIENT UNBLOCK`)。
//! 客户端编号在 Redis 重启或主备切换后会从小号重新分配:会话带上服务器的 run_id,终止前核对。

use std::collections::HashMap;

use redis::Value;

use super::edit::quote;
use super::{RedisSession, info_fields, parse_keyspace};
use crate::error::{CoreError, CoreResult};
use crate::monitor::{
    Activity, KillMode, KillTarget, OWN_CONNECTION, RedisStats, ServerCounters, ServerProcess,
    ServerSnapshot, SlowCommand, clip, gone, now_ms,
};

/// 慢日志最多取这么多条。
const SLOWLOG_ENTRIES: usize = 30;

fn no_client_list(message: &str) -> String {
    if message.starts_with("NOPERM") {
        "这个账号不能执行 CLIENT LIST，看不到客户端列表。".into()
    } else {
        format!("读不了客户端列表（{message}）。")
    }
}

impl RedisSession {
    pub async fn server_snapshot(&self) -> CoreResult<ServerSnapshot> {
        let db = self.default_db;
        let info: String = self.query(db, &redis::cmd("INFO")).await?;
        let taken_at_ms = now_ms();
        let fields = info_fields(&info);
        let run_id = fields.get("run_id").map(|id| id.trim().to_owned());
        // 同一条连接上问 CLIENT ID 和 CLIENT LIST,列表里才认得出哪个是自己。
        let mut pipe = redis::pipe();
        pipe.cmd("CLIENT").arg("ID").cmd("CLIENT").arg("LIST");
        let (processes, limited) = match self.query_pipe::<(i64, String)>(db, &pipe).await {
            Ok((own, list)) => (parse_clients(&list, Some(own), run_id.as_deref()), None),
            // CLIENT ID 要 Redis 5;代理、ACL 也可能只禁了其中一条:单独问 CLIENT LIST,认不出自己就不标。
            Err(CoreError::Database(_)) => {
                match self
                    .query::<String>(db, redis::cmd("CLIENT").arg("LIST"))
                    .await
                {
                    Ok(list) => (parse_clients(&list, None, run_id.as_deref()), None),
                    Err(CoreError::Database(message)) => {
                        (Vec::new(), Some(no_client_list(&message)))
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        // 云上常把 SLOWLOG、CONFIG 关掉:读不到就不显示。
        // 监控自己每次都执行 CLIENT LIST,客户端多时它也会进慢日志:按自己的地址滤掉。
        let own_addr = processes
            .iter()
            .find(|p| p.is_self)
            .and_then(|p| p.host.clone());
        let slowlog = match self
            .query::<Value>(db, redis::cmd("SLOWLOG").arg("GET").arg(SLOWLOG_ENTRIES))
            .await
        {
            Ok(value) => Some(
                parse_slowlog(value)
                    .into_iter()
                    .filter(|entry| own_addr.is_none() || entry.client != own_addr)
                    .collect(),
            ),
            Err(error) => {
                log::debug!("读不了慢日志：{error}");
                None
            }
        };
        let slow_count: Option<u64> = self.query(db, redis::cmd("SLOWLOG").arg("LEN")).await.ok();
        let max_clients = match number(&fields, "maxclients") {
            Some(max) => Some(max),
            None => self
                .query::<Vec<String>>(db, redis::cmd("CONFIG").arg("GET").arg("maxclients"))
                .await
                .ok()
                .and_then(|pair| pair.get(1)?.parse().ok()),
        };
        Ok(ServerSnapshot {
            counters: counters(&fields, max_clients, slow_count),
            processes,
            taken_at_ms,
            limited,
            redis: Some(stats(&info, &fields, slowlog)),
        })
    }

    /// 断开一个客户端,或者解除它的阻塞(BLPOP 这类一直在等的命令马上返回)。
    pub async fn kill_client(&self, target: &KillTarget) -> CoreResult<()> {
        let db = self.default_db;
        // 拿不到 CLIENT ID(老版本、被禁用)也不要紧:CLIENT KILL 默认 SKIPME,杀不到自己。
        if let Ok(own) = self.query::<i64>(db, redis::cmd("CLIENT").arg("ID")).await
            && target.id == own
        {
            return Err(CoreError::InvalidData(OWN_CONNECTION.into()));
        }
        if let Some(expected) = &target.session_start {
            let info: String = self.query(db, redis::cmd("INFO").arg("server")).await?;
            let now = info_fields(&info)
                .get("run_id")
                .map(|id| id.trim().to_owned());
            if now.as_deref() != Some(expected.as_str()) {
                return Err(CoreError::Database(format!(
                    "Redis 重启过或者换了节点，会话 {} 已经不是确认时的那个了，没有动手。",
                    target.id
                )));
            }
        }
        match target.mode {
            KillMode::Connection => {
                let killed: i64 = self
                    .execute(
                        db,
                        redis::cmd("CLIENT").arg("KILL").arg("ID").arg(target.id),
                    )
                    .await?;
                if killed == 0 {
                    return Err(gone(target.id));
                }
            }
            KillMode::Query => {
                let unblocked: i64 = self
                    .execute(db, redis::cmd("CLIENT").arg("UNBLOCK").arg(target.id))
                    .await
                    .map_err(|error| match error {
                        CoreError::Database(message)
                            if message.to_ascii_lowercase().contains("unknown subcommand") =>
                        {
                            CoreError::Unsupported(
                                "这个 Redis 版本不能解除阻塞（CLIENT UNBLOCK 要 5.0 以上），可以断开它的连接"
                                    .into(),
                            )
                        }
                        other => other,
                    })?;
                if unblocked == 0 {
                    return Err(CoreError::Database(format!(
                        "会话 {} 现在没有在等（不在阻塞状态，或者已经断开了），没有动它。",
                        target.id
                    )));
                }
            }
        }
        Ok(())
    }
}

fn number(fields: &HashMap<&str, &str>, name: &str) -> Option<u64> {
    fields.get(name)?.trim().parse().ok()
}

fn counters(
    fields: &HashMap<&str, &str>,
    max_clients: Option<u64>,
    slow_count: Option<u64>,
) -> ServerCounters {
    let get = |name: &str| number(fields, name);
    let hits = get("keyspace_hits");
    let misses = get("keyspace_misses");
    ServerCounters {
        uptime_seconds: get("uptime_in_seconds"),
        connections: get("connected_clients").unwrap_or_default(),
        max_connections: max_clients,
        // Redis 一次只执行一条命令;「在执行」的是阻塞着在等的客户端。
        running: get("blocked_clients").unwrap_or_default(),
        queries: get("total_commands_processed"),
        commits: None,
        rollbacks: None,
        bytes_received: get("total_net_input_bytes"),
        bytes_sent: get("total_net_output_bytes"),
        cache_reads: hits.zip(misses).map(|(h, m)| h + m),
        cache_misses: misses,
        slow_queries: slow_count,
    }
}

fn stats(
    info: &str,
    fields: &HashMap<&str, &str>,
    slowlog: Option<Vec<SlowCommand>>,
) -> RedisStats {
    let (keys, expires) = parse_keyspace(info)
        .values()
        .fold((0, 0), |(k, e), (keys, expires)| (k + keys, e + expires));
    RedisStats {
        memory_used: number(fields, "used_memory"),
        memory_peak: number(fields, "used_memory_peak"),
        memory_max: number(fields, "maxmemory").filter(|&max| max > 0),
        eviction_policy: fields.get("maxmemory_policy").map(|p| p.trim().to_owned()),
        keys,
        expires,
        evicted_keys: number(fields, "evicted_keys"),
        expired_keys: number(fields, "expired_keys"),
        role: fields.get("role").map(|r| r.trim().to_owned()),
        run_id: fields.get("run_id").map(|r| r.trim().to_owned()),
        slowlog,
    }
}

/// `CLIENT LIST` 一行一个客户端,`键=值` 用空格隔开。`own` 是自己的编号(不知道时为 `None`)。
fn parse_clients(list: &str, own: Option<i64>, run_id: Option<&str>) -> Vec<ServerProcess> {
    list.lines()
        .filter_map(|line| {
            let fields: HashMap<&str, &str> = line
                .split(' ')
                .filter_map(|pair| pair.split_once('='))
                .collect();
            let id: i64 = fields.get("id")?.parse().ok()?;
            Some(client(&fields, id, Some(id) == own, run_id))
        })
        .collect()
}

fn client(
    fields: &HashMap<&str, &str>,
    id: i64,
    is_self: bool,
    run_id: Option<&str>,
) -> ServerProcess {
    let text = |name: &str| {
        fields
            .get(name)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let flags = fields.get("flags").copied().unwrap_or_default();
    let (activity, state) = if flags.contains('b') {
        (Activity::Running, "阻塞中")
    } else if flags.contains('x') {
        (Activity::InTransaction, "事务中")
    } else if flags.contains('M') {
        (Activity::Background, "主节点")
    } else if flags.contains('S') {
        (Activity::Background, "从节点")
    } else if flags.contains('O') {
        // MONITOR 是普通客户端,常常正是要断开的那个(它拖慢吞吐):不算服务器自己的。
        (Activity::Idle, "MONITOR")
    } else if flags.contains('P') {
        (Activity::Idle, "订阅中")
    } else {
        (Activity::Idle, "空闲")
    };
    let library = text("lib-name").map(|name| match text("lib-ver") {
        Some(version) => format!("{name} {version}"),
        None => name,
    });
    let detail = [text("name"), library]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    ServerProcess {
        id,
        user: text("user"),
        host: text("addr"),
        database: text("db").map(|db| format!("db{db}")),
        state: Some(state.to_owned()),
        detail: (!detail.is_empty()).then_some(detail),
        seconds: fields.get("idle").and_then(|s| s.parse().ok()),
        // 最近一条命令只有命令名(`client|list` 这样的写法),没有参数。
        query: text("cmd")
            .filter(|cmd| cmd != "NULL")
            .map(|cmd| clip(cmd.replace('|', " ").to_ascii_uppercase())),
        activity,
        // 编号只在这次运行里不重复:终止前核对 run_id,重启或切换过节点就不动手。
        session_start: run_id.map(str::to_owned),
        is_self,
    }
}

/// `SLOWLOG GET`:每条是 [编号, 时刻, 微秒, [参数…], 客户端地址, 客户端名](后两项 Redis 4 起才有)。
fn parse_slowlog(value: Value) -> Vec<SlowCommand> {
    let Value::Array(entries) = value else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let Value::Array(fields) = entry else {
                return None;
            };
            let int = |i: usize| match fields.get(i) {
                Some(Value::Int(n)) => u64::try_from(*n).ok(),
                _ => None,
            };
            let text = |i: usize| match fields.get(i) {
                Some(Value::BulkString(bytes)) if !bytes.is_empty() => {
                    Some(String::from_utf8_lossy(bytes).into_owned())
                }
                Some(Value::SimpleString(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            let command = match fields.get(3) {
                Some(Value::Array(args)) => args
                    .iter()
                    .map(|arg| match arg {
                        Value::BulkString(bytes) => quote(bytes),
                        Value::SimpleString(s) => quote(s.as_bytes()),
                        other => format!("{other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            Some(SlowCommand {
                id: int(0)?,
                at: int(1)?,
                micros: int(2)?,
                command: clip(command),
                client: text(4),
                name: text(5),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_clients_and_what_they_are_doing() {
        let list = "id=3 addr=10.0.0.5:51234 laddr=10.0.0.1:6379 fd=8 name=worker-1 age=600 idle=12 flags=b db=1 sub=0 psub=0 multi=-1 cmd=blpop user=app lib-name=redis-py lib-ver=5.0.1\n\
                    id=7 addr=127.0.0.1:60000 fd=9 name= age=5 idle=0 flags=N db=0 multi=-1 cmd=client|list user=default lib-name=basalt lib-ver=0.3.1\n\
                    id=9 addr=10.0.0.6:50000 fd=10 name= age=5 idle=3 flags=x db=2 multi=2 cmd=set user=default\n\
                    id=11 addr=10.0.0.7:50000 fd=11 name= age=900 idle=900 flags=S db=0 cmd=replconf user=default\n";
        let clients = parse_clients(list, Some(7), Some("abc"));
        assert_eq!(clients.len(), 4);
        let blocked = &clients[0];
        assert_eq!(blocked.activity, Activity::Running);
        assert_eq!(blocked.state.as_deref(), Some("阻塞中"));
        assert_eq!(blocked.detail.as_deref(), Some("worker-1 · redis-py 5.0.1"));
        assert_eq!(blocked.database.as_deref(), Some("db1"));
        assert_eq!(blocked.query.as_deref(), Some("BLPOP"));
        assert_eq!(blocked.seconds, Some(12));
        assert_eq!(blocked.user.as_deref(), Some("app"));
        assert!(clients[1].is_self);
        assert_eq!(clients[1].query.as_deref(), Some("CLIENT LIST"));
        assert_eq!(clients[2].activity, Activity::InTransaction);
        assert_eq!(clients[3].activity, Activity::Background);
        assert_eq!(clients[3].state.as_deref(), Some("从节点"));
        assert_eq!(clients[0].session_start.as_deref(), Some("abc"));
        assert!(parse_clients(list, None, None).iter().all(|c| !c.is_self));
    }

    #[test]
    fn reads_the_slow_log() {
        let entry = |id: i64, args: &[&[u8]], client: &[u8]| {
            Value::Array(vec![
                Value::Int(id),
                Value::Int(1_729_650_000),
                Value::Int(12_500),
                Value::Array(args.iter().map(|a| Value::BulkString(a.to_vec())).collect()),
                Value::BulkString(client.to_vec()),
                Value::BulkString(Vec::new()),
            ])
        };
        let log = parse_slowlog(Value::Array(vec![
            entry(2, &[b"KEYS", b"user:*"], b"10.0.0.5:51234"),
            entry(1, &[b"SET", b"k", b"two words"], b""),
        ]));
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].command, "KEYS user:*");
        assert_eq!(log[0].micros, 12_500);
        assert_eq!(log[0].client.as_deref(), Some("10.0.0.5:51234"));
        assert_eq!(log[0].name, None);
        assert_eq!(log[1].command, "SET k \"two words\"");
        assert!(parse_slowlog(Value::Nil).is_empty());
    }

    #[test]
    fn counts_keys_and_hit_rate_from_info() {
        let info = "# Server\r\nuptime_in_seconds:3600\r\n# Clients\r\nconnected_clients:5\r\nblocked_clients:1\r\nmaxclients:10000\r\n\
                    # Memory\r\nused_memory:1048576\r\nused_memory_peak:2097152\r\nmaxmemory:0\r\nmaxmemory_policy:noeviction\r\n\
                    # Stats\r\ntotal_commands_processed:900\r\nkeyspace_hits:90\r\nkeyspace_misses:10\r\nevicted_keys:0\r\n\
                    # Replication\r\nrole:master\r\n# Keyspace\r\ndb0:keys=12,expires=3,avg_ttl=0\r\ndb5:keys=1,expires=0,avg_ttl=0\r\n";
        let fields = info_fields(info);
        let counters = counters(&fields, number(&fields, "maxclients"), Some(4));
        assert_eq!(counters.connections, 5);
        assert_eq!(counters.max_connections, Some(10_000));
        assert_eq!(counters.running, 1);
        assert_eq!(counters.queries, Some(900));
        assert_eq!(
            (counters.cache_reads, counters.cache_misses),
            (Some(100), Some(10))
        );
        assert_eq!(counters.slow_queries, Some(4));
        let stats = stats(info, &fields, None);
        assert_eq!((stats.keys, stats.expires), (13, 3));
        assert_eq!(stats.memory_used, Some(1_048_576));
        assert_eq!(stats.memory_max, None, "0 是没设上限");
        assert_eq!(stats.role.as_deref(), Some("master"));
    }
}
