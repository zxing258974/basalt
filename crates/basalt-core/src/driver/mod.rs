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
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::config::{ConnectionConfig, DriverKind, Endpoint, SslSettings};
use crate::error::{CoreError, CoreResult};
use crate::tunnel::{KnownHosts, LocalEndpoint, SshParams, Tunnel};

/// 建立连接(含握手与认证)的最长等待时间。
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 从连接池取连接的最长等待时间。
pub(crate) const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(15);
/// 桌面客户端同时跑的查询很少,每个池保持小规模。
pub(crate) const MAX_POOL_CONNECTIONS: u32 = 4;
pub(crate) const APPLICATION_NAME: &str = "basalt";

/// 打开一个连接所需的全部参数。`Debug` 不输出密码。
/// 经 SSH 隧道时 `host`、`port` 是从 SSH 服务器那边看的数据库地址。
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectParams {
    pub driver: DriverKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub database: Option<String>,
    pub file_path: Option<String>,
    pub ssl: SslSettings,
    pub ssh: Option<SshParams>,
}

impl ConnectParams {
    /// `ssh_secret` 是 SSH 密码或私钥口令,`known_hosts` 记着信任过的 SSH 服务器;没开隧道时都用不上。
    pub fn from_config(
        config: &ConnectionConfig,
        password: Option<String>,
        ssh_secret: Option<String>,
        known_hosts: &KnownHosts,
    ) -> Self {
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
                ssl: config.ssl.clone(),
                ssh: config
                    .ssh
                    .as_ref()
                    .map(|tunnel| SshParams::from_config(tunnel, ssh_secret, known_hosts.clone())),
            },
            Endpoint::File { path } => Self {
                driver: config.driver,
                host: String::new(),
                port: 0,
                username: String::new(),
                password: None,
                database: None,
                file_path: Some(path.clone()),
                ssl: SslSettings::default(),
                ssh: None,
            },
        }
    }

    /// 网络连接的通用写法,测试和导入导出的夹具用。
    pub fn network(driver: DriverKind, host: &str, port: u16, username: &str) -> Self {
        Self {
            driver,
            host: host.to_owned(),
            port,
            username: username.to_owned(),
            password: None,
            database: None,
            file_path: None,
            ssl: SslSettings::default(),
            ssh: None,
        }
    }
}

/// 驱动实际要连的地方。
#[derive(Debug)]
pub(crate) struct Route {
    /// 直连时就是原参数;经隧道的 TCP 入口时主机、端口换成本机的;经 Unix 套接字时不变,
    /// 主机名留给 TLS 核对证书用。
    pub params: ConnectParams,
    /// 经隧道的 Unix 套接字:PostgreSQL 给目录,MySQL 给文件。
    pub socket: Option<PathBuf>,
    pub tunnel: Option<Tunnel>,
}

/// 直连就是原参数;有 SSH 隧道时先建隧道,改连它在本机的入口。
/// 先查证书文件在不在,免得 TLS 握手时报一句看不懂的 I/O 错误。
pub(crate) async fn route(params: &ConnectParams) -> CoreResult<Route> {
    check_ssl_files(&params.ssl)?;
    let Some(ssh) = &params.ssh else {
        return Ok(Route {
            params: params.clone(),
            socket: None,
            tunnel: None,
        });
    };
    let postgres = params.driver == DriverKind::Postgres;
    // PostgreSQL 按「目录/.s.PGSQL.端口」找套接字。
    let socket_name = if postgres {
        format!(".s.PGSQL.{}", params.port)
    } else {
        "mysqld.sock".to_owned()
    };
    let tunnel = Tunnel::open(ssh, &params.host, params.port, &socket_name).await?;
    let direct = ConnectParams {
        ssh: None,
        ..params.clone()
    };
    let (params, socket) = match tunnel.endpoint() {
        LocalEndpoint::Unix(file) => {
            let socket = if postgres {
                file.parent().map(Path::to_path_buf)
            } else {
                Some(file.clone())
            };
            (direct, socket)
        }
        LocalEndpoint::Tcp(address) => (
            ConnectParams {
                host: Ipv4Addr::LOCALHOST.to_string(),
                port: address.port(),
                ..direct
            },
            None,
        ),
    };
    Ok(Route {
        params,
        socket,
        tunnel: Some(tunnel),
    })
}

/// 经隧道连库失败:换成隧道记下的真正原因,再把隧道关掉。
pub(crate) async fn abandon(tunnel: Option<Tunnel>, error: CoreError) -> CoreError {
    let Some(tunnel) = tunnel else {
        return error;
    };
    let error = tunnel.explain(error);
    tunnel.close().await;
    error
}

fn check_ssl_files(ssl: &SslSettings) -> CoreResult<()> {
    for (label, path) in ssl.files() {
        if !Path::new(path).is_file() {
            return Err(CoreError::File(format!("找不到{label}文件：{path}")));
        }
    }
    Ok(())
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
            .field("ssl", &self.ssl)
            .field("ssh", &self.ssh)
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
    use crate::config::{SshAuthMethod, SshTunnel, SslMode};
    use crate::tunnel::SshAuth;

    fn known_hosts() -> KnownHosts {
        KnownHosts::new("/tmp/basalt-test-known_hosts")
    }

    fn network_config(ssh: Option<SshTunnel>) -> ConnectionConfig {
        ConnectionConfig {
            id: "n".into(),
            name: "n".into(),
            group: None,
            driver: DriverKind::Postgres,
            endpoint: Endpoint::Network {
                host: "10.0.0.5".into(),
                port: 5432,
                username: "app".into(),
                database: Some("shop".into()),
            },
            save_password: true,
            ssh,
            ssl: SslSettings {
                mode: SslMode::Required,
                ..SslSettings::default()
            },
        }
    }

    #[test]
    fn debug_output_redacts_password() {
        let params = ConnectParams {
            password: Some("hunter2".into()),
            ..ConnectParams::network(DriverKind::Mysql, "db", 3306, "root")
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
            ssh: None,
            ssl: SslSettings::default(),
        };
        let params = ConnectParams::from_config(&config, Some("x".into()), None, &known_hosts());
        assert_eq!(params.file_path.as_deref(), Some("/tmp/a.db"));
        assert_eq!(params.password, None);
        assert_eq!(params.ssh, None);
    }

    #[test]
    fn network_config_carries_ssl_and_tunnel_credentials() {
        let direct = ConnectParams::from_config(&network_config(None), None, None, &known_hosts());
        assert_eq!(direct.ssl.mode, SslMode::Required);
        assert_eq!(direct.ssh, None);

        let tunnel = SshTunnel {
            host: "jump".into(),
            port: 2222,
            username: "deploy".into(),
            auth: SshAuthMethod::PrivateKey,
            private_key_path: Some("/keys/id".into()),
            save_secret: true,
            jump_hosts: None,
        };
        let params = ConnectParams::from_config(
            &network_config(Some(tunnel)),
            Some("db-pass".into()),
            Some("key-pass".into()),
            &known_hosts(),
        );
        let ssh = params.ssh.clone().unwrap();
        assert_eq!((ssh.host.as_str(), ssh.port), ("jump", 2222));
        assert_eq!(
            ssh.auth,
            SshAuth::PrivateKey {
                path: "/keys/id".into(),
                passphrase: Some("key-pass".into()),
            }
        );
        let debug = format!("{params:?}");
        assert!(
            !debug.contains("key-pass") && !debug.contains("db-pass"),
            "{debug}"
        );
    }

    #[tokio::test]
    async fn route_without_tunnel_is_direct_and_checks_certificate_files() {
        let params = ConnectParams::network(DriverKind::Mysql, "db", 3306, "root");
        let direct = route(&params).await.unwrap();
        assert_eq!(direct.params, params);
        assert!(direct.tunnel.is_none() && direct.socket.is_none());

        let missing = ConnectParams {
            ssl: SslSettings {
                mode: SslMode::VerifyCa,
                ca_path: Some("/nonexistent/basalt-ca.pem".into()),
                ..SslSettings::default()
            },
            ..params.clone()
        };
        let error = route(&missing).await.unwrap_err();
        assert_eq!(error.code(), "file");
        assert!(error.to_string().contains("CA 证书"), "{error}");

        // 禁用 SSL 时不管证书路径。
        let disabled = ConnectParams {
            ssl: SslSettings {
                mode: SslMode::Disabled,
                ..missing.ssl.clone()
            },
            ..params
        };
        assert!(route(&disabled).await.is_ok());
    }

    #[tokio::test]
    async fn abandon_without_tunnel_keeps_the_error() {
        let error = abandon(None, CoreError::Timeout).await;
        assert_eq!(error.code(), "timeout");
    }

    #[test]
    fn non_empty_drops_blank_strings() {
        assert_eq!(non_empty(Some("  ".into())), None);
        assert_eq!(non_empty(Some("x".into())), Some("x".into()));
        assert_eq!(non_empty(None), None);
    }
}
