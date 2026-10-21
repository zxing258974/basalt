//! 复制表要用到的执行方式:逐条执行并带回每条影响的行数、在一个事务里执行,
//! 以及 SQLite 建表、建索引语句的原文与 PostgreSQL 标识列。语句由 [`crate::table_ops`] 生成。

use sqlx::Row;

use super::mysql::MysqlSession;
use super::postgres::PgSession;
use super::sqlite::SqliteSession;
use super::{finish_transaction, statement_failed, table_not_found};
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, quote_ident};
use crate::table::TableRef;

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
}
