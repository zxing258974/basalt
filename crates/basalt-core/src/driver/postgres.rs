//! PostgreSQL。一个连接只能连一个库,所以按库名懒建连接池。

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::{ConnectOptions, Connection, Row};
use tokio::sync::{Mutex, OnceCell};

use super::{
    ACQUIRE_TIMEOUT, APPLICATION_NAME, CONNECT_TIMEOUT, ConnectParams, MAX_POOL_CONNECTIONS,
    ServerInfo, TableInfo, TableKind, non_empty,
};
use crate::error::{CoreError, CoreResult};

const DEFAULT_DATABASE: &str = "postgres";

/// 每个库一个格子:建池时只锁这个库自己的格子,不会挡住其他库和 close()。
type PoolCell = Arc<OnceCell<PgPool>>;

pub struct PgSession {
    base: PgConnectOptions,
    default_database: String,
    pools: Mutex<HashMap<String, PoolCell>>,
    server: ServerInfo,
}

pub(crate) fn connect_options(params: &ConnectParams) -> PgConnectOptions {
    let options = PgConnectOptions::new()
        .host(&params.host)
        .port(params.port)
        .username(&params.username)
        .application_name(APPLICATION_NAME);
    match &params.password {
        Some(password) => options.password(password),
        None => options,
    }
}

fn initial_database(params: &ConnectParams) -> String {
    params
        .database
        .clone()
        .unwrap_or_else(|| DEFAULT_DATABASE.to_owned())
}

/// 用单条连接探测(连接被拒绝、库不存在等立即报错)并读出版本。
/// 握手和查询整体受超时约束:连接池代理(如 pgbouncer)可能握手很快、查询却一直挂着。
async fn probe(options: &PgConnectOptions) -> CoreResult<String> {
    let attempt = async {
        let mut connection = options.connect().await?;
        let version: String = sqlx::query_scalar("SELECT current_setting('server_version')")
            .fetch_one(&mut connection)
            .await?;
        if let Err(error) = connection.close().await {
            log::warn!("关闭 PostgreSQL 探测连接失败：{error}");
        }
        Ok::<_, CoreError>(version)
    };
    tokio::time::timeout(CONNECT_TIMEOUT, attempt)
        .await
        .map_err(|_| CoreError::Timeout)?
}

/// 探测成功后再懒建连接池。返回池和服务器版本。
///
/// sqlx 建连时把会话时区固定成 UTC;这里改成本机时区,带时区的时间按本机时区
/// 显示,筛选条件里写的时间字面量也按本机时区理解,两边一致。
async fn connect_pool(options: PgConnectOptions) -> CoreResult<(PgPool, String)> {
    let version = probe(&options).await?;
    let time_zone = local_time_zone();
    let pool = PgPoolOptions::new()
        .max_connections(MAX_POOL_CONNECTIONS)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .after_connect(move |connection, _| {
            let time_zone = time_zone.clone();
            Box::pin(async move {
                if let Some(zone) = time_zone {
                    // 服务器不认识这个时区名时保持 UTC,不让连接失败。
                    if let Err(error) = sqlx::query("SELECT set_config('TimeZone', $1, false)")
                        .bind(&zone)
                        .execute(&mut *connection)
                        .await
                    {
                        log::warn!("无法把会话时区设为 {zone}：{error}");
                    }
                }
                Ok(())
            })
        })
        .connect_lazy_with(options);
    Ok((pool, version))
}

/// 本机的 IANA 时区名:优先 TZ 环境变量,其次 /etc/localtime 指向的 zoneinfo 文件。
fn local_time_zone() -> Option<String> {
    if let Ok(zone) = std::env::var("TZ") {
        let zone = zone.trim_start_matches(':').trim();
        if !zone.is_empty() {
            return Some(zone.to_owned());
        }
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_string_lossy();
    target
        .split_once("zoneinfo/")
        .map(|(_, zone)| zone.to_owned())
        .filter(|zone| !zone.is_empty())
}

impl PgSession {
    pub async fn open(params: &ConnectParams) -> CoreResult<Self> {
        let base = connect_options(params);
        let default_database = initial_database(params);
        let (pool, version) = connect_pool(base.clone().database(&default_database)).await?;
        Ok(Self {
            base,
            pools: Mutex::new(HashMap::from([(
                default_database.clone(),
                Arc::new(OnceCell::from(pool)),
            )])),
            default_database,
            server: ServerInfo {
                product: "PostgreSQL".into(),
                version,
            },
        })
    }

    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    /// 连接配置里的库,没填时是 postgres。
    pub(super) fn default_database(&self) -> &str {
        &self.default_database
    }

    /// 取某个库的池,第一次用时才建。建失败不留缓存,下次会重试。
    pub(super) async fn pool(&self, database: &str) -> CoreResult<PgPool> {
        let cell = self
            .pools
            .lock()
            .await
            .entry(database.to_owned())
            .or_default()
            .clone();
        let pool = cell
            .get_or_try_init(|| async {
                let (pool, _) = connect_pool(self.base.clone().database(database)).await?;
                Ok::<_, CoreError>(pool)
            })
            .await?;
        Ok(pool.clone())
    }

    pub async fn list_databases(&self) -> CoreResult<Vec<String>> {
        let pool = self.pool(&self.default_database).await?;
        let names = sqlx::query_scalar(
            "SELECT datname::text FROM pg_database
             WHERE datallowconn AND NOT datistemplate
             ORDER BY datname",
        )
        .fetch_all(&pool)
        .await?;
        Ok(names)
    }

    /// 隐藏 pg_catalog、pg_toast、临时 schema 与 information_schema。
    pub async fn list_schemas(&self, database: &str) -> CoreResult<Vec<String>> {
        let pool = self.pool(database).await?;
        let names = sqlx::query_scalar(
            r"SELECT nspname::text FROM pg_namespace
              WHERE nspname NOT LIKE 'pg\_%' AND nspname <> 'information_schema'
              ORDER BY nspname",
        )
        .fetch_all(&pool)
        .await?;
        Ok(names)
    }

    pub async fn list_tables(&self, database: &str, schema: &str) -> CoreResult<Vec<TableInfo>> {
        let pool = self.pool(database).await?;
        let rows = sqlx::query(
            "SELECT c.relname::text AS name,
                    c.relkind::text AS relkind,
                    c.reltuples::bigint AS estimated_rows,
                    pg_table_size(c.oid) AS data_bytes,
                    pg_indexes_size(c.oid) AS index_bytes,
                    obj_description(c.oid, 'pg_class') AS comment
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
             ORDER BY c.relname",
        )
        .bind(schema)
        .fetch_all(&pool)
        .await?;

        rows.iter()
            .map(|row| {
                let kind = relation_kind(&row.try_get::<String, _>("relkind")?);
                let is_view = kind == TableKind::View;
                let estimated: Option<i64> = row.try_get("estimated_rows")?;
                Ok(TableInfo {
                    name: row.try_get("name")?,
                    kind,
                    engine: None,
                    rows: if is_view {
                        None
                    } else {
                        known_row_estimate(estimated)
                    },
                    data_bytes: if is_view {
                        None
                    } else {
                        row.try_get("data_bytes")?
                    },
                    index_bytes: if is_view {
                        None
                    } else {
                        row.try_get("index_bytes")?
                    },
                    created_at: None,
                    updated_at: None,
                    comment: non_empty(row.try_get("comment")?),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }

    pub async fn close(&self) {
        let cells: Vec<PoolCell> = self.pools.lock().await.drain().map(|(_, c)| c).collect();
        for pool in cells.iter().filter_map(|cell| cell.get()) {
            pool.close().await;
        }
    }
}

fn relation_kind(relkind: &str) -> TableKind {
    match relkind {
        "v" => TableKind::View,
        "m" => TableKind::MaterializedView,
        "f" => TableKind::ForeignTable,
        _ => TableKind::Table,
    }
}

/// PG 14 起,从未 ANALYZE 过的表 reltuples 为 -1,表示「未知」。
fn known_row_estimate(estimate: Option<i64>) -> Option<i64> {
    estimate.filter(|n| *n >= 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DriverKind;

    fn params(database: Option<&str>) -> ConnectParams {
        ConnectParams {
            driver: DriverKind::Postgres,
            host: "pg.local".into(),
            port: 6543,
            username: "app".into(),
            password: None,
            database: database.map(str::to_owned),
            file_path: None,
        }
    }

    #[test]
    fn options_carry_endpoint_and_application_name() {
        let options = connect_options(&params(None));
        assert_eq!(options.get_host(), "pg.local");
        assert_eq!(options.get_port(), 6543);
        assert_eq!(options.get_username(), "app");
        assert_eq!(options.get_application_name(), Some(APPLICATION_NAME));
    }

    #[test]
    fn initial_database_defaults_to_postgres() {
        assert_eq!(initial_database(&params(None)), "postgres");
        assert_eq!(initial_database(&params(Some("shop"))), "shop");
    }

    #[test]
    fn maps_relkind_and_unknown_estimates() {
        assert_eq!(relation_kind("r"), TableKind::Table);
        assert_eq!(relation_kind("p"), TableKind::Table);
        assert_eq!(relation_kind("v"), TableKind::View);
        assert_eq!(relation_kind("m"), TableKind::MaterializedView);
        assert_eq!(relation_kind("f"), TableKind::ForeignTable);
        assert_eq!(known_row_estimate(Some(-1)), None);
        assert_eq!(known_row_estimate(Some(0)), Some(0));
        assert_eq!(known_row_estimate(None), None);
    }
}
