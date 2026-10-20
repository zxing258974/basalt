//! 三种数据库的连接与元数据查询。各驱动只负责「怎么连、怎么查」,
//! 会话生命周期见 [`crate::session`]。

pub(crate) mod mysql;
pub(crate) mod postgres;
pub(crate) mod sqlite;

use std::fmt;
use std::time::Duration;

use serde::Serialize;

use crate::config::{ConnectionConfig, DriverKind, Endpoint};

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
