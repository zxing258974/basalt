//! SQLite。一个文件一个会话;附加库(ATTACH)按 database 名区分。

use std::path::Path;

use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use super::{ACQUIRE_TIMEOUT, ConnectParams, ServerInfo, TableInfo, TableKind};
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, quote_ident};

/// SQLite 写入是串行的,池开大了只会多等锁。
const MAX_SQLITE_CONNECTIONS: u32 = 2;

pub struct SqliteSession {
    pool: SqlitePool,
    server: ServerInfo,
}

impl SqliteSession {
    pub async fn open(params: &ConnectParams) -> CoreResult<Self> {
        let path = params
            .file_path
            .as_deref()
            .ok_or_else(|| CoreError::Database("没有指定数据库文件".into()))?;
        if !Path::new(path).is_file() {
            return Err(CoreError::Database(format!("找不到数据库文件：{path}")));
        }
        // 不设置 journal_mode:那是持久化在文件里的,打开别人的库不应改动它。
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_SQLITE_CONNECTIONS)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect_with(options)
            .await?;
        // sqlite_version() 对任何文件都能成功,读一次 sqlite_master 才能确认它真是数据库。
        if let Err(error) = sqlx::query("SELECT count(*) FROM sqlite_master")
            .fetch_one(&pool)
            .await
        {
            pool.close().await;
            return Err(CoreError::Database(format!(
                "不是有效的 SQLite 数据库文件：{path}（{error}）"
            )));
        }
        let version: String = sqlx::query_scalar("SELECT sqlite_version()")
            .fetch_one(&pool)
            .await?;
        Ok(Self {
            pool,
            server: ServerInfo {
                product: "SQLite".into(),
                version,
            },
        })
    }

    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    pub async fn list_databases(&self) -> CoreResult<Vec<String>> {
        let rows = sqlx::query("PRAGMA database_list")
            .fetch_all(&self.pool)
            .await?;
        let names = rows
            .iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names.into_iter().filter(|n| n != "temp").collect())
    }

    pub async fn list_tables(&self, database: &str) -> CoreResult<Vec<TableInfo>> {
        let sql = format!(
            r"SELECT name, type FROM {}.sqlite_master
              WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\_%' ESCAPE '\'
              ORDER BY name",
            quote_ident(Dialect::Sqlite, database)
        );
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                let kind = match row.try_get::<String, _>("type")?.as_str() {
                    "view" => TableKind::View,
                    _ => TableKind::Table,
                };
                Ok(TableInfo {
                    name: row.try_get("name")?,
                    kind,
                    engine: None,
                    rows: None,
                    data_bytes: None,
                    index_bytes: None,
                    created_at: None,
                    updated_at: None,
                    comment: None,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
