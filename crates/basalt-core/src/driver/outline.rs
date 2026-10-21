//! SQL 助手的表结构概要查询,见 [`crate::llm::outline`]。

use sqlx::Row;

use super::mysql::MysqlSession;
use super::postgres::PgSession;
use super::sqlite::SqliteSession;
use crate::error::CoreResult;
use crate::llm::outline::{MAX_COLUMNS, OutlineColumn, OutlineTable, group};
use crate::sql::{Dialect, quote_ident};

fn non_empty(text: Option<String>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty())
}

impl MysqlSession {
    pub(crate) async fn schema_outline(&self, database: &str) -> CoreResult<Vec<OutlineTable>> {
        let rows = sqlx::query(
            "SELECT CAST(c.TABLE_NAME AS CHAR) AS table_name, CAST(t.TABLE_TYPE AS CHAR) AS table_type,
                    CAST(t.TABLE_COMMENT AS CHAR) AS table_comment, CAST(c.COLUMN_NAME AS CHAR) AS name,
                    CAST(c.COLUMN_TYPE AS CHAR) AS data_type, CAST(c.COLUMN_KEY AS CHAR) AS column_key,
                    CAST(c.IS_NULLABLE AS CHAR) AS nullable, CAST(c.COLUMN_COMMENT AS CHAR) AS comment
             FROM information_schema.COLUMNS c
             JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME
             WHERE c.TABLE_SCHEMA = ?
             ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION
             LIMIT ?",
        )
        .bind(database)
        .bind(MAX_COLUMNS)
        .fetch_all(&self.pool)
        .await?;
        let rows = rows
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("table_name")?,
                    row.try_get::<Option<String>, _>("table_type")?.as_deref() == Some("VIEW"),
                    non_empty(row.try_get("table_comment")?),
                    OutlineColumn {
                        name: row.try_get("name")?,
                        data_type: row.try_get("data_type")?,
                        primary_key: row.try_get::<Option<String>, _>("column_key")?.as_deref()
                            == Some("PRI"),
                        nullable: row.try_get::<Option<String>, _>("nullable")?.as_deref()
                            == Some("YES"),
                        comment: non_empty(row.try_get("comment")?),
                    },
                ))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(group(rows))
    }
}

impl PgSession {
    pub(crate) async fn schema_outline(
        &self,
        database: &str,
        schema: &str,
    ) -> CoreResult<Vec<OutlineTable>> {
        let pool = self.pool(database).await?;
        let rows = sqlx::query(
            "SELECT c.relname::text AS table_name, c.relkind::text AS kind,
                    obj_description(c.oid, 'pg_class') AS table_comment, a.attname::text AS name,
                    format_type(a.atttypid, a.atttypmod) AS data_type,
                    COALESCE(a.attnum = ANY(i.indkey), false) AS primary_key,
                    NOT a.attnotnull AS nullable, col_description(c.oid, a.attnum) AS comment
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
             LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary
             WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
             ORDER BY c.relname, a.attnum
             LIMIT $2",
        )
        .bind(schema)
        .bind(MAX_COLUMNS)
        .fetch_all(&pool)
        .await?;
        let rows = rows
            .iter()
            .map(|row| {
                let kind: String = row.try_get("kind")?;
                Ok((
                    row.try_get::<String, _>("table_name")?,
                    kind == "v" || kind == "m",
                    non_empty(row.try_get("table_comment")?),
                    OutlineColumn {
                        name: row.try_get("name")?,
                        data_type: row.try_get("data_type")?,
                        primary_key: row.try_get("primary_key")?,
                        nullable: row.try_get("nullable")?,
                        comment: non_empty(row.try_get("comment")?),
                    },
                ))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(group(rows))
    }
}

impl SqliteSession {
    pub(crate) async fn schema_outline(&self, database: &str) -> CoreResult<Vec<OutlineTable>> {
        let schema = quote_ident(Dialect::Sqlite, database);
        let rows = sqlx::query(&format!(
            "SELECT m.name AS table_name, m.type AS kind, p.name AS name, p.type AS data_type,
                    p.pk AS pk, p.\"notnull\" AS not_null
             FROM {schema}.sqlite_master m
             JOIN pragma_table_info(m.name, ?) p
             WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%'
             ORDER BY m.name, p.cid
             LIMIT ?"
        ))
        .bind(database)
        .bind(MAX_COLUMNS)
        .fetch_all(&self.pool)
        .await?;
        let rows = rows
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("table_name")?,
                    row.try_get::<String, _>("kind")? == "view",
                    None,
                    OutlineColumn {
                        name: row.try_get("name")?,
                        data_type: row.try_get("data_type")?,
                        primary_key: row.try_get::<i64, _>("pk")? > 0,
                        // INTEGER PRIMARY KEY 在 pragma 里报可空(SQLite 的老毛病),实际不会是 NULL。
                        nullable: row.try_get::<i64, _>("not_null")? == 0
                            && row.try_get::<i64, _>("pk")? == 0,
                        comment: None,
                    },
                ))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(group(rows))
    }
}
