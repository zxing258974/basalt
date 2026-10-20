//! MySQL 与 MariaDB(同一协议)。

use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};
use sqlx::{ConnectOptions, Connection, Row};

use super::{
    ACQUIRE_TIMEOUT, CONNECT_TIMEOUT, ConnectParams, MAX_POOL_CONNECTIONS, ServerInfo, TableInfo,
    TableKind, non_empty,
};
use crate::error::{CoreError, CoreResult};

pub struct MysqlSession {
    pub(super) pool: MySqlPool,
    server: ServerInfo,
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
    let options = match &params.password {
        Some(password) => options.password(password),
        None => options,
    };
    match &params.database {
        Some(database) => options.database(database),
        None => options,
    }
}

impl MysqlSession {
    pub async fn open(params: &ConnectParams) -> CoreResult<Self> {
        let options = connect_options(params);
        // 先用单条连接探测:连接池遇到「连接被拒绝」会一直重试到超时,探测连接则立即报错。
        // 握手和版本查询整体受超时约束。
        let probe = async {
            let mut connection = options.connect().await?;
            let version: String = sqlx::query_scalar("SELECT CAST(VERSION() AS CHAR)")
                .fetch_one(&mut connection)
                .await?;
            if let Err(error) = connection.close().await {
                log::warn!("关闭 MySQL 探测连接失败：{error}");
            }
            Ok::<_, CoreError>(version)
        };
        let version = tokio::time::timeout(CONNECT_TIMEOUT, probe)
            .await
            .map_err(|_| CoreError::Timeout)??;
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
    }
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
