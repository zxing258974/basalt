//! 各家查「别的表上引用这张表的外键」。类型见 [`crate::table_ops::references`]。

use sqlx::Row;

use super::mysql::MysqlSession;
use super::postgres::PgSession;
use super::sqlite::SqliteSession;
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, dynamic, quote_ident};
use crate::table::TableRef;
use crate::table_ops::references::{OnDelete, ReferencingKey};

impl MysqlSession {
    pub(crate) async fn referencing_keys(
        &self,
        table: &TableRef,
    ) -> CoreResult<Vec<ReferencingKey>> {
        let rows = sqlx::query(
            "SELECT CAST(CONSTRAINT_SCHEMA AS CHAR) AS owner_database, CAST(TABLE_NAME AS CHAR) AS owner,
                    CAST(CONSTRAINT_NAME AS CHAR) AS name, CAST(DELETE_RULE AS CHAR) AS rule
             FROM information_schema.REFERENTIAL_CONSTRAINTS
             WHERE UNIQUE_CONSTRAINT_SCHEMA = ? AND REFERENCED_TABLE_NAME = ?
               AND NOT (CONSTRAINT_SCHEMA = UNIQUE_CONSTRAINT_SCHEMA AND TABLE_NAME = REFERENCED_TABLE_NAME)
             ORDER BY CONSTRAINT_SCHEMA, TABLE_NAME, CONSTRAINT_NAME",
        )
        .bind(&table.database)
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(ReferencingKey {
                    table: TableRef {
                        database: row.try_get("owner_database")?,
                        schema: None,
                        table: row.try_get("owner")?,
                    },
                    name: Some(row.try_get("name")?),
                    on_delete: OnDelete::parse(&row.try_get::<String, _>("rule")?),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }
}

impl PgSession {
    /// 分区表的每个分区上各有一份继承来的外键(conparentid 不为 0),只取最上面那份。
    pub(crate) async fn referencing_keys(
        &self,
        table: &TableRef,
    ) -> CoreResult<Vec<ReferencingKey>> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let rows = sqlx::query(
            "SELECT n.nspname::text AS owner_schema, c.relname::text AS owner,
                    con.conname::text AS name, con.confdeltype::text AS rule
             FROM pg_constraint con
             JOIN pg_class c ON c.oid = con.conrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_class t ON t.oid = con.confrelid
             JOIN pg_namespace tn ON tn.oid = t.relnamespace
             WHERE con.contype = 'f' AND con.conparentid = 0 AND con.conrelid <> con.confrelid
               AND tn.nspname = $1 AND t.relname = $2
             ORDER BY 1, 2, 3",
        )
        .bind(schema)
        .bind(&table.table)
        .fetch_all(&pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(ReferencingKey {
                    table: TableRef {
                        database: table.database.clone(),
                        schema: Some(row.try_get("owner_schema")?),
                        table: row.try_get("owner")?,
                    },
                    name: Some(row.try_get("name")?),
                    on_delete: OnDelete::parse(&row.try_get::<String, _>("rule")?),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }
}

impl SqliteSession {
    /// 多列外键在 `pragma_foreign_key_list` 里每列一行,按外键的 id 合成一条。
    pub(crate) async fn referencing_keys(
        &self,
        table: &TableRef,
    ) -> CoreResult<Vec<ReferencingKey>> {
        let master = format!(
            "{}.sqlite_master",
            quote_ident(Dialect::Sqlite, &table.database)
        );
        let rows = sqlx::query(dynamic(format!(
            "SELECT m.name AS owner, f.on_delete AS rule
             FROM {master} m, pragma_foreign_key_list(m.name, ?1) f
             WHERE m.type = 'table' AND f.\"table\" = ?2 COLLATE NOCASE AND m.name <> ?2 COLLATE NOCASE
             GROUP BY m.name, f.id ORDER BY m.name, f.id"
        )))
        .bind(&table.database)
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(ReferencingKey {
                    table: TableRef {
                        database: table.database.clone(),
                        schema: None,
                        table: row.try_get("owner")?,
                    },
                    name: None,
                    on_delete: OnDelete::parse(&row.try_get::<String, _>("rule")?),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }
}
