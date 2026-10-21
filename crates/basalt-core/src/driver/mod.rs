//! 三种数据库的连接与元数据查询。各驱动只负责「怎么连、怎么查」,
//! 会话生命周期见 [`crate::session`]。

mod dump;
pub(crate) mod editor;
pub(crate) mod mysql;
mod mysql_data;
mod mysql_design;
mod mysql_query;
mod outline;
mod pg_binary;
pub(crate) mod postgres;
mod postgres_data;
mod postgres_design;
mod postgres_query;
mod references;
pub(crate) mod sqlite;
mod sqlite_data;
mod sqlite_ddl;
mod sqlite_design;
mod sqlite_query;
mod stream;
mod table_info;
mod table_ops;

use std::fmt;
use std::time::Duration;

use serde::Serialize;

use crate::config::{ConnectionConfig, DriverKind, Endpoint};
use crate::error::{CoreError, CoreResult};

/// 建立连接(含握手与认证)的最长等待时间。
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 从连接池取连接的最长等待时间。
pub(crate) const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(15);
/// 桌面客户端同时跑的查询很少,每个池保持小规模。
pub(crate) const MAX_POOL_CONNECTIONS: u32 = 4;
pub(crate) const APPLICATION_NAME: &str = "basalt";

/// 打开一个连接所需的全部参数。`Debug` 不输出密码。
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectParams {
    pub driver: DriverKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub database: Option<String>,
    pub file_path: Option<String>,
}

impl ConnectParams {
    pub fn from_config(config: &ConnectionConfig, password: Option<String>) -> Self {
        match &config.endpoint {
            Endpoint::Network {
                host,
                port,
                username,
                database,
            } => Self {
                driver: config.driver,
                host: host.clone(),
                port: *port,
                username: username.clone(),
                password,
                database: database.clone(),
                file_path: None,
            },
            Endpoint::File { path } => Self {
                driver: config.driver,
                host: String::new(),
                port: 0,
                username: String::new(),
                password: None,
                database: None,
                file_path: Some(path.clone()),
            },
        }
    }
}

impl fmt::Debug for ConnectParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectParams")
            .field("driver", &self.driver)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("database", &self.database)
            .field("file_path", &self.file_path)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    /// MySQL / MariaDB / PostgreSQL / SQLite
    pub product: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TableKind {
    Table,
    View,
    MaterializedView,
    ForeignTable,
}

/// 对象列表页的一行。各库拿不到的字段为 `None`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
    pub engine: Option<String>,
    /// 估算行数(MySQL 的 TABLE_ROWS、PG 的 reltuples),不是精确计数。
    pub rows: Option<i64>,
    pub data_bytes: Option<i64>,
    pub index_bytes: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comment: Option<String>,
}

pub(crate) fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// 第 `index` 条语句执行失败:给数据库原文加上序号和「已回滚」。
pub(crate) fn statement_failed(index: usize, error: sqlx::Error) -> CoreError {
    match CoreError::from(error) {
        CoreError::Database(message) | CoreError::Authentication(message) => {
            CoreError::Database(format!(
                "第 {} 条语句执行失败，所有修改都已回滚：{message}",
                index + 1
            ))
        }
        other => other,
    }
}

/// 按主键定位的语句必须恰好命中一行;否则说明这一行已经变了,整批回滚。
pub(crate) fn check_affected(index: usize, affected: u64) -> CoreResult<()> {
    if affected == 1 {
        return Ok(());
    }
    Err(CoreError::RowMismatch(format!(
        "第 {} 条语句影响了 {affected} 行（应为 1 行）：这一行可能已被别人修改或删除。所有修改都已回滚。",
        index + 1
    )))
}

/// 成功就提交,失败就显式回滚再把原错误交出去。
pub(crate) async fn finish_transaction<DB: sqlx::Database>(
    transaction: sqlx::Transaction<'_, DB>,
    result: CoreResult<u64>,
) -> CoreResult<u64> {
    match result {
        Ok(affected) => {
            // COMMIT 本身失败(如连接在途中断开)时,数据库里到底有没有生效说不准,不能说「已回滚」。
            transaction.commit().await.map_err(|error| {
                CoreError::Database(format!(
                    "提交（COMMIT）时出错，无法确定修改是否已经生效，请刷新后核对：{}",
                    CoreError::from(error)
                ))
            })?;
            Ok(affected)
        }
        Err(error) => {
            if let Err(rollback) = transaction.rollback().await {
                log::error!("回滚失败，连接断开后数据库会自行回滚：{rollback}");
            }
            Err(error)
        }
    }
}

/// 找不到表(或没有权限看到它的列)。
pub(crate) fn table_not_found(name: &str) -> CoreError {
    CoreError::Database(format!("找不到表「{name}」，可能已被删除或没有权限。"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_redacts_password() {
        let params = ConnectParams {
            driver: DriverKind::Mysql,
            host: "db".into(),
            port: 3306,
            username: "root".into(),
            password: Some("hunter2".into()),
            database: None,
            file_path: None,
        };
        let debug = format!("{params:?}");
        assert!(!debug.contains("hunter2"));
        assert!(debug.contains("***"));
    }

    #[test]
    fn file_endpoint_ignores_password() {
        let config = ConnectionConfig {
            id: "s".into(),
            name: "s".into(),
            group: None,
            driver: DriverKind::Sqlite,
            endpoint: Endpoint::File {
                path: "/tmp/a.db".into(),
            },
            save_password: false,
        };
        let params = ConnectParams::from_config(&config, Some("x".into()));
        assert_eq!(params.file_path.as_deref(), Some("/tmp/a.db"));
        assert_eq!(params.password, None);
    }

    #[test]
    fn non_empty_drops_blank_strings() {
        assert_eq!(non_empty(Some("  ".into())), None);
        assert_eq!(non_empty(Some("x".into())), Some("x".into()));
        assert_eq!(non_empty(None), None);
    }
}
