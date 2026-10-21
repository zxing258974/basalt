//! 表级操作要用到的执行方式:逐条执行并带回每条影响的行数、在一个事务里执行、不进事务执行(VACUUM),
//! 以及 SQLite 建表、建索引语句的原文、PostgreSQL 标识列和维护前后的大小。语句由 [`crate::table_ops`] 生成。

use sqlx::{Executor, Row};

use super::mysql::MysqlSession;
use super::postgres::PgSession;
use super::sqlite::SqliteSession;
use super::{finish_transaction, statement_failed, table_not_found};
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, quote_ident};
use crate::table::TableRef;
use crate::table_ops::maintenance::MaintenanceMessage;
use crate::transfer::dump::SequenceColumn;

/// 一个文本格:列是二进制排序规则时 MySQL 以字节返回。
fn text_at(row: &sqlx::mysql::MySqlRow, index: usize) -> Result<String, sqlx::Error> {
    match row.try_get::<Option<String>, _>(index) {
        Ok(text) => Ok(text.unwrap_or_default()),
        Err(_) => Ok(row
            .try_get::<Option<Vec<u8>>, _>(index)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()),
    }
}

impl MysqlSession {
    /// 逐条执行(DDL 会隐式提交,放不进事务),返回每条影响的行数;第 n 条(从 0 数)失败时返回 `(n, 错误)`。
    pub(crate) async fn execute_each(
        &self,
        statements: &[String],
    ) -> Result<Vec<u64>, (usize, CoreError)> {
        let mut connection = self.pool.acquire().await.map_err(|e| (0, e.into()))?;
        let mut counts = Vec::with_capacity(statements.len());
        for (index, sql) in statements.iter().enumerate() {
            let done = sqlx::query(sql)
                .persistent(false)
                .execute(&mut *connection)
                .await
                .map_err(|error| (index, CoreError::from(error)))?;
            counts.push(done.rows_affected());
        }
        Ok(counts)
    }

    /// 表上的触发器,按执行顺序拼成 CREATE TRIGGER(不带 DEFINER,还原时归执行的用户;表名不带库名,和转储的建表语句一致)。
    pub(crate) async fn trigger_statements(&self, table: &TableRef) -> CoreResult<Vec<String>> {
        let rows = sqlx::query(
            "SELECT CAST(TRIGGER_NAME AS CHAR) AS name, CAST(ACTION_TIMING AS CHAR) AS timing,
                    CAST(EVENT_MANIPULATION AS CHAR) AS event, CAST(ACTION_STATEMENT AS CHAR) AS body
             FROM information_schema.TRIGGERS
             WHERE EVENT_OBJECT_SCHEMA = ? AND EVENT_OBJECT_TABLE = ?
             ORDER BY ACTION_TIMING, EVENT_MANIPULATION, ACTION_ORDER",
        )
        .bind(&table.database)
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?;
        let d = Dialect::Mysql;
        rows.iter()
            .map(|row| {
                let name: String = row.try_get("name")?;
                let timing: String = row.try_get("timing")?;
                let event: String = row.try_get("event")?;
                let body: String = row.try_get("body")?;
                Ok(format!(
                    "CREATE TRIGGER {} {timing} {event} ON {} FOR EACH ROW {}",
                    quote_ident(d, &name),
                    quote_ident(d, &table.table),
                    body.trim().trim_end_matches(';')
                ))
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }

    /// ANALYZE / CHECK / OPTIMIZE / REPAIR TABLE 返回的消息(Table、Op、Msg_type、Msg_text 四列)。
    /// 走文本协议:CHECK TABLE 不能预备执行。
    pub(crate) async fn maintenance_messages(
        &self,
        sql: &str,
    ) -> CoreResult<Vec<MaintenanceMessage>> {
        let mut connection = self.pool.acquire().await?;
        let rows = (&mut *connection).fetch_all(sql).await?;
        let messages = rows
            .iter()
            .map(|row| {
                Ok(MaintenanceMessage {
                    level: text_at(row, 2)?.to_lowercase(),
                    text: text_at(row, 3)?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?;
        Ok(messages)
    }
}

impl PgSession {
    /// 在一个事务里执行,任何一条失败就整体回滚。返回每条影响的行数。
    pub(crate) async fn execute_in_transaction(
        &self,
        database: &str,
        statements: &[String],
    ) -> CoreResult<Vec<u64>> {
        let pool = self.pool(database).await?;
        let mut transaction = pool.begin().await?;
        let mut counts = Vec::with_capacity(statements.len());
        let result = async {
            for (index, sql) in statements.iter().enumerate() {
                let done = sqlx::query(sql)
                    .persistent(false)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|error| statement_failed(index, error))?;
                counts.push(done.rows_affected());
            }
            Ok(0)
        }
        .await;
        finish_transaction(transaction, result).await?;
        Ok(counts)
    }

    /// 表上的标识列(GENERATED … AS IDENTITY):复制数据后要把新表的序列推到最大值。
    pub(crate) async fn identity_columns(&self, table: &TableRef) -> CoreResult<Vec<String>> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let names = sqlx::query_scalar(
            "SELECT a.attname::text FROM pg_attribute a
             JOIN pg_class c ON c.oid = a.attrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
               AND a.attidentity <> ''
             ORDER BY a.attnum",
        )
        .bind(schema)
        .bind(&table.table)
        .fetch_all(&pool)
        .await?;
        Ok(names)
    }

    /// 表上用户建的触发器(不含外键约束自带的内部触发器)。
    pub(crate) async fn trigger_statements(&self, table: &TableRef) -> CoreResult<Vec<String>> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let statements = sqlx::query_scalar(
            "SELECT pg_get_triggerdef(t.oid, true) FROM pg_trigger t
             JOIN pg_class c ON c.oid = t.tgrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = $2 AND NOT t.tgisinternal
             ORDER BY t.tgname",
        )
        .bind(schema)
        .bind(&table.table)
        .fetch_all(&pool)
        .await?;
        Ok(statements)
    }

    /// 带序列的列(标识列、serial 列),转储时要重建序列、把它推到最大值。
    pub(crate) async fn sequence_columns(
        &self,
        table: &TableRef,
    ) -> CoreResult<Vec<SequenceColumn>> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let rows = sqlx::query(
            "SELECT * FROM (
               SELECT a.attname::text AS name, a.attidentity <> '' AS identity, a.attnum,
                      pg_get_serial_sequence(format('%I.%I', n.nspname, c.relname), a.attname) AS sequence
               FROM pg_attribute a
               JOIN pg_class c ON c.oid = a.attrelid
               JOIN pg_namespace n ON n.oid = c.relnamespace
               WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
             ) s WHERE sequence IS NOT NULL ORDER BY attnum",
        )
        .bind(schema)
        .bind(&table.table)
        .fetch_all(&pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(SequenceColumn {
                    column: row.try_get("name")?,
                    sequence: row.try_get("sequence")?,
                    identity: row.try_get("identity")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }

    /// 能不能对这张表做 VACUUM、ANALYZE:要是表的所有者、库的所有者或超级用户,PostgreSQL 17 起也可以是
    /// 有 MAINTAIN 权限的用户。没有权限时数据库不报错,只发一条警告就跳过这张表,所以要先问。
    pub(crate) async fn can_maintain(&self, table: &TableRef) -> CoreResult<bool> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let allowed: Option<bool> = sqlx::query_scalar(
            "SELECT CASE WHEN current_setting('server_version_num')::int >= 170000
                         THEN has_table_privilege(c.oid, 'MAINTAIN')
                         ELSE pg_has_role(c.relowner, 'USAGE')
                              OR pg_has_role(d.datdba, 'USAGE')
                              OR (SELECT rolsuper FROM pg_roles WHERE rolname = current_user)
                    END
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             CROSS JOIN pg_database d
             WHERE n.nspname = $1 AND c.relname = $2 AND d.datname = current_database()",
        )
        .bind(schema)
        .bind(&table.table)
        .fetch_optional(&pool)
        .await?;
        allowed.ok_or_else(|| table_not_found(&table.table))
    }

    /// VACUUM 不能在事务块里执行:单独一条、走简单查询协议。
    pub(crate) async fn execute_outside_transaction(
        &self,
        database: &str,
        sql: &str,
    ) -> CoreResult<()> {
        let pool = self.pool(database).await?;
        let mut connection = pool.acquire().await?;
        (&mut *connection).execute(sql).await?;
        Ok(())
    }

    /// 表连同索引、TOAST 的大小(字节)。
    pub(crate) async fn total_relation_size(&self, table: &TableRef) -> CoreResult<u64> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let size: Option<i64> = sqlx::query_scalar(
            "SELECT pg_total_relation_size(c.oid) FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = $2",
        )
        .bind(schema)
        .bind(&table.table)
        .fetch_optional(&pool)
        .await?;
        let size = size.ok_or_else(|| table_not_found(&table.table))?;
        Ok(u64::try_from(size).unwrap_or(0))
    }
}

impl SqliteSession {
    /// 在一个事务里执行,任何一条失败就整体回滚。返回每条影响的行数。
    pub(crate) async fn execute_in_transaction(
        &self,
        statements: &[String],
    ) -> CoreResult<Vec<u64>> {
        let mut transaction = self.pool.begin().await?;
        let mut counts = Vec::with_capacity(statements.len());
        let result = async {
            for (index, sql) in statements.iter().enumerate() {
                let done = sqlx::query(sql)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|error| statement_failed(index, error))?;
                counts.push(done.rows_affected());
            }
            Ok(0)
        }
        .await;
        finish_transaction(transaction, result).await?;
        Ok(counts)
    }

    /// `sqlite_master` 里存的建表语句,和这张表上显式建的索引(名字、建索引语句)。
    /// 主键、UNIQUE 约束自带的索引没有语句,建表时会自动建出来。
    pub(crate) async fn schema_sql(
        &self,
        table: &TableRef,
    ) -> CoreResult<(String, Vec<(String, String)>)> {
        let master = format!(
            "{}.sqlite_master",
            quote_ident(Dialect::Sqlite, &table.database)
        );
        let table_sql: Option<Option<String>> = sqlx::query_scalar(&format!(
            "SELECT sql FROM {master} WHERE type = 'table' AND name = ?"
        ))
        .bind(&table.table)
        .fetch_optional(&self.pool)
        .await?;
        let table_sql = table_sql
            .flatten()
            .ok_or_else(|| table_not_found(&table.table))?;
        let indexes = sqlx::query(&format!(
            "SELECT name, sql FROM {master}
             WHERE type = 'index' AND tbl_name = ? AND sql IS NOT NULL ORDER BY name"
        ))
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| Ok((row.try_get("name")?, row.try_get("sql")?)))
        .collect::<Result<_, sqlx::Error>>()?;
        Ok((table_sql, indexes))
    }

    /// VACUUM 不能在事务里执行。
    pub(crate) async fn execute_outside_transaction(&self, sql: &str) -> CoreResult<()> {
        let mut connection = self.pool.acquire().await?;
        (&mut *connection).execute(sql).await?;
        Ok(())
    }

    /// 库文件的大小:页数乘页大小(字节)。
    pub(crate) async fn database_size(&self, database: &str) -> CoreResult<u64> {
        let schema = quote_ident(Dialect::Sqlite, database);
        let pages: i64 = sqlx::query_scalar(&format!("PRAGMA {schema}.page_count"))
            .fetch_one(&self.pool)
            .await?;
        let page_size: i64 = sqlx::query_scalar(&format!("PRAGMA {schema}.page_size"))
            .fetch_one(&self.pool)
            .await?;
        Ok(u64::try_from(pages.saturating_mul(page_size)).unwrap_or(0))
    }

    /// 库里有没有 `sqlite_sequence`(建过 AUTOINCREMENT 表才有)。
    pub(crate) async fn has_sqlite_sequence(&self, database: &str) -> CoreResult<bool> {
        let master = format!("{}.sqlite_master", quote_ident(Dialect::Sqlite, database));
        let found: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT 1 FROM {master} WHERE type = 'table' AND name = 'sqlite_sequence'"
        ))
        .fetch_optional(&self.pool)
        .await?;
        Ok(found.is_some())
    }
}
