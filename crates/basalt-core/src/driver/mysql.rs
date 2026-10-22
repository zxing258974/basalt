//! MySQL 与 MariaDB(同一协议)。

use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlSslMode};
use sqlx::{ConnectOptions, Connection, Row};

use super::{
    ACQUIRE_TIMEOUT, CONNECT_TIMEOUT, ConnectParams, MAX_POOL_CONNECTIONS, ServerInfo, TableInfo,
    TableKind, non_empty,
};
use crate::config::{SslMode, SslSettings};
use crate::error::{CoreError, CoreResult};
use crate::tunnel::Tunnel;

pub struct MysqlSession {
    pub(super) pool: MySqlPool,
    server: ServerInfo,
    /// 经 SSH 隧道时,池里的连接都连它的本机端口;池关掉之后再关它。
    tunnel: Option<Tunnel>,
}

/// sqlx 默认在建连后改会话:sql_mode 追加 PIPES_AS_CONCAT(`||` 变成字符串拼接)与
/// NO_ENGINE_SUBSTITUTION,time_zone 设成 '+00:00'(DEFAULT CURRENT_TIMESTAMP 存成 UTC、
/// TIMESTAMP 按 UTC 显示)。客户端要和服务器默认行为一致,这三项都关掉。
pub(crate) fn connect_options(params: &ConnectParams) -> MySqlConnectOptions {
    let options = MySqlConnectOptions::new()
        .host(&params.host)
        .port(params.port)
        .username(&params.username)
        .charset("utf8mb4")
        .pipes_as_concat(false)
        .no_engine_substitution(false)
        .timezone(None);
    // 空密码要当「不带密码」:sqlx 拿空串也会算一遍摘要发过去,空密码账号的服务器只认空的认证数据。
    let options = match &params.password {
        Some(password) if !password.is_empty() => options.password(password),
        _ => options,
    };
    let options = with_ssl(options, &params.ssl);
    match &params.database {
        Some(database) => options.database(database),
        None => options,
    }
}

fn with_ssl(options: MySqlConnectOptions, ssl: &SslSettings) -> MySqlConnectOptions {
    let mode = match ssl.mode {
        SslMode::Disabled => return options.ssl_mode(MySqlSslMode::Disabled),
        SslMode::Preferred => MySqlSslMode::Preferred,
        // 和 mysql 客户端一样:「必须」又给了 CA 证书,就按它校验。
        SslMode::Required if ssl.ca_path.is_some() => MySqlSslMode::VerifyCa,
        SslMode::Required => MySqlSslMode::Required,
        SslMode::VerifyCa => MySqlSslMode::VerifyCa,
        SslMode::VerifyIdentity => MySqlSslMode::VerifyIdentity,
    };
    let options = options.ssl_mode(mode);
    let options = match &ssl.ca_path {
        Some(path) => options.ssl_ca(path),
        None => options,
    };
    match (&ssl.client_cert_path, &ssl.client_key_path) {
        (Some(cert), Some(key)) => options.ssl_client_cert(cert).ssl_client_key(key),
        _ => options,
    }
}

impl MysqlSession {
    pub async fn open(params: &ConnectParams) -> CoreResult<Self> {
        let super::Route {
            params: direct,
            socket,
            tunnel,
        } = super::route(params).await?;
        let options = connect_options(&direct);
        let options = match &socket {
            Some(path) => options.socket(path),
            None => options,
        };
        let version = match probe(&options).await {
            Ok(version) => version,
            Err(error) => return Err(super::abandon(tunnel, error).await),
        };
        let pool = MySqlPoolOptions::new()
            .max_connections(MAX_POOL_CONNECTIONS)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect_lazy_with(options);

        let product = if version.to_ascii_lowercase().contains("mariadb") {
            "MariaDB"
        } else {
            "MySQL"
        };
        Ok(Self {
            pool,
            server: ServerInfo {
                product: product.into(),
                version,
            },
            tunnel,
        })
    }

    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    pub async fn list_databases(&self) -> CoreResult<Vec<String>> {
        // information_schema 的列在 MySQL 8 里可能是二进制排序规则,统一 CAST 成字符。
        let names = sqlx::query_scalar(
            "SELECT CAST(SCHEMA_NAME AS CHAR) FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(names)
    }

    pub async fn list_tables(&self, database: &str) -> CoreResult<Vec<TableInfo>> {
        let rows = sqlx::query(
            "SELECT CAST(TABLE_NAME AS CHAR) AS name,
                    CAST(TABLE_TYPE AS CHAR) AS kind,
                    CAST(ENGINE AS CHAR) AS engine,
                    CAST(TABLE_ROWS AS SIGNED) AS table_rows,
                    CAST(DATA_LENGTH AS SIGNED) AS data_bytes,
                    CAST(INDEX_LENGTH AS SIGNED) AS index_bytes,
                    CAST(CREATE_TIME AS CHAR) AS created_at,
                    CAST(UPDATE_TIME AS CHAR) AS updated_at,
                    CAST(TABLE_COMMENT AS CHAR) AS comment
             FROM information_schema.TABLES
             WHERE TABLE_SCHEMA = ?
             ORDER BY TABLE_NAME",
        )
        .bind(database)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let kind = table_kind(&row.try_get::<String, _>("kind")?);
                let comment: Option<String> = row.try_get("comment")?;
                Ok(TableInfo {
                    name: row.try_get("name")?,
                    kind,
                    engine: row.try_get("engine")?,
                    rows: row.try_get("table_rows")?,
                    data_bytes: row.try_get("data_bytes")?,
                    index_bytes: row.try_get("index_bytes")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                    comment: visible_comment(kind, comment),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }

    pub async fn close(&self) {
        self.pool.close().await;
        if let Some(tunnel) = &self.tunnel {
            tunnel.close().await;
        }
    }
}

/// 先用单条连接探测:连接池遇到「连接被拒绝」会一直重试到超时,探测连接则立即报错。
/// 握手和版本查询整体受超时约束。
async fn probe(options: &MySqlConnectOptions) -> CoreResult<String> {
    let attempt = async {
        let mut connection = options.connect().await?;
        let version: String = sqlx::query_scalar("SELECT CAST(VERSION() AS CHAR)")
            .fetch_one(&mut connection)
            .await?;
        if let Err(error) = connection.close().await {
            log::warn!("关闭 MySQL 探测连接失败：{error}");
        }
        Ok::<_, CoreError>(version)
    };
    tokio::time::timeout(CONNECT_TIMEOUT, attempt)
        .await
        .map_err(|_| CoreError::Timeout)?
}

fn table_kind(table_type: &str) -> TableKind {
    match table_type {
        "VIEW" | "SYSTEM VIEW" => TableKind::View,
        _ => TableKind::Table,
    }
}

/// 视图的 TABLE_COMMENT 固定是字面量 `VIEW`,对用户没有意义。
fn visible_comment(kind: TableKind, comment: Option<String>) -> Option<String> {
    non_empty(comment).filter(|c| !(kind == TableKind::View && c == "VIEW"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DriverKind;

    #[test]
    fn options_carry_endpoint_and_optional_database() {
        let params = ConnectParams {
            driver: DriverKind::Mysql,
            host: "db.local".into(),
            port: 3307,
            username: "app".into(),
            password: Some("pw".into()),
            database: Some("shop".into()),
            file_path: None,
            ssl: crate::config::SslSettings::default(),
            ssh: None,
        };
        let options = connect_options(&params);
        assert_eq!(options.get_host(), "db.local");
        assert_eq!(options.get_port(), 3307);
        assert_eq!(options.get_username(), "app");
        assert_eq!(options.get_database(), Some("shop"));

        let without_db = connect_options(&ConnectParams {
            database: None,
            ..params
        });
        assert_eq!(without_db.get_database(), None);
    }

    #[test]
    fn ssl_settings_map_to_mysql_modes() {
        let base = ConnectParams::network(DriverKind::Mysql, "db", 3306, "root");
        assert!(matches!(
            connect_options(&base).get_ssl_mode(),
            MySqlSslMode::Preferred
        ));
        for (mode, expected) in [
            (SslMode::Disabled, MySqlSslMode::Disabled),
            // 给了 CA 的「必须」按 CA 校验。
            (SslMode::Required, MySqlSslMode::VerifyCa),
            (SslMode::VerifyCa, MySqlSslMode::VerifyCa),
            (SslMode::VerifyIdentity, MySqlSslMode::VerifyIdentity),
        ] {
            let params = ConnectParams {
                ssl: SslSettings {
                    mode,
                    ca_path: Some("/certs/ca.pem".into()),
                    ..SslSettings::default()
                },
                ..base.clone()
            };
            let options = connect_options(&params);
            assert_eq!(
                std::mem::discriminant(&options.get_ssl_mode()),
                std::mem::discriminant(&expected),
                "{mode:?}"
            );
        }
        let required = ConnectParams {
            ssl: SslSettings {
                mode: SslMode::Required,
                ..SslSettings::default()
            },
            ..base
        };
        assert!(matches!(
            connect_options(&required).get_ssl_mode(),
            MySqlSslMode::Required
        ));
    }

    #[test]
    fn maps_table_types_and_hides_view_placeholder_comment() {
        assert_eq!(table_kind("BASE TABLE"), TableKind::Table);
        assert_eq!(table_kind("SYSTEM VIEW"), TableKind::View);
        assert_eq!(table_kind("SEQUENCE"), TableKind::Table);
        assert_eq!(visible_comment(TableKind::View, Some("VIEW".into())), None);
        assert_eq!(
            visible_comment(TableKind::Table, Some("VIEW".into())),
            Some("VIEW".into())
        );
        assert_eq!(visible_comment(TableKind::Table, Some("".into())), None);
    }
}
