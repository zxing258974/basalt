//! Redis 单机。连接流自己建(直连、SSH 隧道、TLS,见 [`link`]),交给 redis 的多路复用连接。
//! `SELECT` 只对当前连接生效:每个库号各开一条连接,几个标签页看不同的库互不干扰。

mod error;
mod link;
mod tls;

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use redis::aio::MultiplexedConnection;
use redis::{Cmd, FromRedisValue};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::driver::{ConnectParams, ServerInfo, abandon, route};
use crate::error::{CoreError, CoreResult};
use crate::tunnel::Tunnel;
use error::is_broken;
use link::{Link, Live};

/// 浏览用的连接:一条命令最多等这么久,免得界面一直转圈。
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
/// 库不多于这些时全部列出;更多时(阿里云标准版有 256 个)只列出有键的库、0 号库和配置里的默认库。
const LIST_ALL_UP_TO: u32 = 16;
/// 拿不到 `CONFIG GET databases`(云上常禁用 CONFIG)时按 Redis 的默认值算。
const DEFAULT_DATABASES: u32 = 16;

/// 一个库:键数和设了过期时间的键数(INFO keyspace)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedisDatabase {
    pub index: u32,
    pub keys: u64,
    pub expires: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedisKeyspace {
    /// 服务器上一共有几个库。
    pub count: u32,
    /// 对象树里列出的库。
    pub databases: Vec<RedisDatabase>,
    /// 连接配置里的库号。
    pub default_db: u32,
}

pub struct RedisSession {
    link: Link,
    server: ServerInfo,
    default_db: u32,
    connections: Mutex<HashMap<u32, Live>>,
    tunnel: Option<Tunnel>,
}

impl RedisSession {
    pub async fn open(params: &ConnectParams) -> CoreResult<Self> {
        let default_db = default_db(params)?;
        let route = route(params).await?;
        match Self::connect(params, &route, default_db).await {
            Ok((link, live, server)) => Ok(Self {
                link,
                server,
                default_db,
                connections: Mutex::new(HashMap::from([(default_db, live)])),
                tunnel: route.tunnel,
            }),
            Err(error) => Err(abandon(route.tunnel, error).await),
        }
    }

    async fn connect(
        params: &ConnectParams,
        route: &crate::driver::Route,
        db: u32,
    ) -> CoreResult<(Link, Live, ServerInfo)> {
        let link = Link::new(params, route)?;
        let mut live = link.connect(db, Some(RESPONSE_TIMEOUT)).await?;
        let checked = async {
            let info: String = redis::cmd("INFO")
                .arg("server")
                .query_async(&mut live.conn)
                .await?;
            server_info(&info)
        }
        .await;
        match checked {
            Ok(server) => Ok((link, live, server)),
            Err(error) => {
                live.driver.abort();
                Err(error)
            }
        }
    }

    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    pub fn default_db(&self) -> u32 {
        self.default_db
    }

    /// 有哪些库、各有多少键。
    pub async fn keyspace(&self) -> CoreResult<RedisKeyspace> {
        let db = self.default_db;
        let info: String = self.query(db, redis::cmd("INFO").arg("keyspace")).await?;
        let counts = parse_keyspace(&info);
        let configured = match self
            .query::<Vec<String>>(db, redis::cmd("CONFIG").arg("GET").arg("databases"))
            .await
        {
            Ok(pair) => pair
                .get(1)
                .and_then(|n| n.parse().ok())
                .unwrap_or(DEFAULT_DATABASES),
            Err(_) => DEFAULT_DATABASES,
        };
        let highest = counts.keys().copied().chain([db]).max().unwrap_or(0);
        let count = configured.max(highest + 1);
        let shown: BTreeSet<u32> = if count <= LIST_ALL_UP_TO {
            (0..count).collect()
        } else {
            counts.keys().copied().chain([0, db]).collect()
        };
        let databases = shown
            .into_iter()
            .map(|index| {
                let (keys, expires) = counts.get(&index).copied().unwrap_or_default();
                RedisDatabase {
                    index,
                    keys,
                    expires,
                }
            })
            .collect();
        Ok(RedisKeyspace {
            count,
            databases,
            default_db: db,
        })
    }

    /// 只读的命令:连接断了换一条再试一次。
    pub(crate) async fn query<T: FromRedisValue>(&self, db: u32, cmd: &Cmd) -> CoreResult<T> {
        let mut conn = self.conn(db).await?;
        match cmd
            .query_async::<T>(&mut conn)
            .await
            .map_err(CoreError::from)
        {
            Err(error) if is_broken(&error) => {
                self.reset(db).await;
                let mut conn = self.conn(db).await?;
                Ok(cmd.query_async(&mut conn).await?)
            }
            other => other,
        }
    }

    async fn conn(&self, db: u32) -> CoreResult<MultiplexedConnection> {
        let mut connections = self.connections.lock().await;
        if let Some(live) = connections.get(&db)
            && !live.driver.is_finished()
        {
            return Ok(live.conn.clone());
        }
        let live = self.link.connect(db, Some(RESPONSE_TIMEOUT)).await?;
        let conn = live.conn.clone();
        if let Some(old) = connections.insert(db, live) {
            old.driver.abort();
        }
        Ok(conn)
    }

    async fn reset(&self, db: u32) {
        if let Some(old) = self.connections.lock().await.remove(&db) {
            old.driver.abort();
        }
    }

    pub async fn close(&self) {
        for (_, live) in self.connections.lock().await.drain() {
            live.driver.abort();
        }
        if let Some(tunnel) = &self.tunnel {
            tunnel.close().await;
        }
    }
}

impl std::fmt::Debug for RedisSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RedisSession({} {})",
            self.server.product, self.server.version
        )
    }
}

/// 连接配置里的库号,留空是 0。
fn default_db(params: &ConnectParams) -> CoreResult<u32> {
    match params
        .database
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        None => Ok(0),
        Some(db) => db
            .parse()
            .map_err(|_| CoreError::Database(format!("库号「{db}」不是 0 或正整数"))),
    }
}

/// INFO 的回复:`# 小节` 和 `键:值` 一行一个。
fn info_fields(info: &str) -> HashMap<&str, &str> {
    info.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.trim_end_matches('\r').split_once(':'))
        .collect()
}

fn server_info(info: &str) -> CoreResult<ServerInfo> {
    let fields = info_fields(info);
    match fields.get("redis_mode").copied() {
        Some("cluster") => {
            return Err(CoreError::Unsupported(
                "这是 Redis 集群的节点。Basalt 目前只支持单机 Redis（云上集群版用代理地址连接即可）".into(),
            ));
        }
        Some("sentinel") => {
            return Err(CoreError::Unsupported(
                "这是 Redis 哨兵（Sentinel）。Basalt 目前只支持单机 Redis，请直接连它管理的主节点"
                    .into(),
            ));
        }
        _ => {}
    }
    let (product, version) = match (fields.get("valkey_version"), fields.get("redis_version")) {
        (Some(version), _) => ("Valkey", *version),
        (None, Some(version)) => ("Redis", *version),
        (None, None) => ("Redis", ""),
    };
    Ok(ServerInfo {
        product: product.into(),
        version: version.trim().into(),
    })
}

/// `db0:keys=12,expires=3,avg_ttl=0` → 库号 → (键数, 设了过期的键数)。
fn parse_keyspace(info: &str) -> HashMap<u32, (u64, u64)> {
    info_fields(info)
        .into_iter()
        .filter_map(|(name, stats)| {
            let index = name.strip_prefix("db")?.parse().ok()?;
            let stat = |key: &str| {
                stats
                    .split(',')
                    .filter_map(|pair| pair.split_once('='))
                    .find(|(k, _)| *k == key)
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0)
            };
            Some((index, (stat("keys"), stat("expires"))))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_product_and_version_and_refuses_clusters() {
        let redis = "# Server\r\nredis_version:7.4.1\r\nredis_mode:standalone\r\n";
        assert_eq!(
            server_info(redis).unwrap(),
            ServerInfo {
                product: "Redis".into(),
                version: "7.4.1".into()
            }
        );
        let valkey =
            "# Server\r\nredis_version:7.2.4\r\nvalkey_version:8.0.1\r\nredis_mode:standalone\r\n";
        assert_eq!(server_info(valkey).unwrap().product, "Valkey");
        assert_eq!(server_info(valkey).unwrap().version, "8.0.1");
        assert!(server_info("redis_version:7.0.0\r\nredis_mode:cluster\r\n").is_err());
        assert!(server_info("redis_version:7.0.0\r\nredis_mode:sentinel\r\n").is_err());
    }

    #[test]
    fn reads_key_counts_per_database() {
        let info = "# Keyspace\r\ndb0:keys=12,expires=3,avg_ttl=0\r\ndb5:keys=1,expires=0,avg_ttl=0,subexpiry=0\r\n";
        let counts = parse_keyspace(info);
        assert_eq!(counts.get(&0), Some(&(12, 3)));
        assert_eq!(counts.get(&5), Some(&(1, 0)));
        assert_eq!(counts.len(), 2);
        assert!(parse_keyspace("# Keyspace\r\n").is_empty());
    }

    #[test]
    fn database_number_defaults_to_zero() {
        let mut params = ConnectParams::network(crate::config::DriverKind::Redis, "h", 6379, "");
        assert_eq!(default_db(&params).unwrap(), 0);
        params.database = Some(" 3 ".into());
        assert_eq!(default_db(&params).unwrap(), 3);
        params.database = Some("x".into());
        assert!(default_db(&params).is_err());
    }
}
