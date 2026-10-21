//! 表级操作要用到的执行方式:逐条执行并带回每条影响的行数、在一个事务里执行、不进事务执行(VACUUM),
//! 以及 SQLite 建表、建索引语句的原文、PostgreSQL 标识列、维护的权限与前后大小。语句由 [`crate::table_ops`] 生成;
//! 转储要读的建表语句、触发器、序列在 [`super::dump`]。

use sqlx::{Executor, Row};

use super::mysql::MysqlSession;
use super::postgres::PgSession;
use super::sqlite::SqliteSession;
use super::{finish_transaction, statement_failed, table_not_found};
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, quote_ident};
use crate::table::TableRef;
use crate::table_ops::maintenance::MaintenanceMessage;

/// 表级操作等锁最多等几秒。别的会话(比如查询页里没提交的事务)占着这张表时,MySQL 的 RENAME / DROP
/// 默认要等一年(lock_wait_timeout)、PostgreSQL 一直等下去,对话框就一直卡着。
pub(crate) const LOCK_WAIT_SECONDS: u64 = 5;

/// 等锁超时:MySQL 是 1205(元数据锁、行锁都是它),PostgreSQL 是 55P03。
fn lock_busy(error: &sqlx::Error) -> bool {
    let Some(database) = error.as_database_error() else {
        return false;
    };
    if let Some(mysql) = database.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() {
        return mysql.number() == 1205;
    }
    database.code().as_deref() == Some("55P03")
}

pub(crate) fn table_busy() -> CoreError {
    CoreError::Database(format!(
        "这张表正被别的会话占用（比如查询页里没提交的事务），等了 {LOCK_WAIT_SECONDS} 秒还没轮到。\
         请先提交或回滚那边的事务再试。"
    ))
}

fn exec_error(error: sqlx::Error) -> CoreError {
    if lock_busy(&error) {
        table_busy()
    } else {
        error.into()
    }
}

/// MySQL 连接上临时缩短等锁时间,用完换回默认值;换不回去就让这条连接关掉,不把设置留给连接池里的别人。
fn mysql_short_locks() -> String {
    format!(
        "SET SESSION lock_wait_timeout = {LOCK_WAIT_SECONDS}, innodb_lock_wait_timeout = {LOCK_WAIT_SECONDS}"
    )
}
const MYSQL_DEFAULT_LOCKS: &str =
    "SET SESSION lock_wait_timeout = DEFAULT, innodb_lock_wait_timeout = DEFAULT";

async fn restore_mysql_locks(connection: &mut sqlx::pool::PoolConnection<sqlx::MySql>) {
    if let Err(error) = (&mut **connection).execute(MYSQL_DEFAULT_LOCKS).await {
        log::warn!("恢复 MySQL 的等锁时间失败，这条连接不再放回连接池：{error}");
        connection.close_on_drop();
    }
}

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
        (&mut *connection)
            .execute(mysql_short_locks().as_str())
            .await
            .map_err(|e| (0, e.into()))?;
        let mut counts = Vec::with_capacity(statements.len());
        let mut result = Ok(());
        for (index, sql) in statements.iter().enumerate() {
            match sqlx::query(sql)
                .persistent(false)
                .execute(&mut *connection)
                .await
            {
                Ok(done) => counts.push(done.rows_affected()),
                Err(error) => {
                    result = Err((index, exec_error(error)));
                    break;
                }
            }
        }
        restore_mysql_locks(&mut connection).await;
        result.map(|()| counts)
    }

    /// ANALYZE / CHECK / OPTIMIZE / REPAIR TABLE 返回的消息(Table、Op、Msg_type、Msg_text 四列)。
    /// 走文本协议:CHECK TABLE 不能预备执行。
    pub(crate) async fn maintenance_messages(
        &self,
        sql: &str,
    ) -> CoreResult<Vec<MaintenanceMessage>> {
        let mut connection = self.pool.acquire().await?;
        (&mut *connection)
            .execute(mysql_short_locks().as_str())
            .await?;
        let fetched = (&mut *connection).fetch_all(sql).await;
        restore_mysql_locks(&mut connection).await;
        let rows = fetched.map_err(exec_error)?;
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
            sqlx::query(&format!("SET LOCAL lock_timeout = '{LOCK_WAIT_SECONDS}s'"))
                .execute(&mut *transaction)
                .await?;
            for (index, sql) in statements.iter().enumerate() {
                let done = sqlx::query(sql)
                    .persistent(false)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|error| match lock_busy(&error) {
                        true => table_busy(),
                        false => statement_failed(index, error),
                    })?;
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

    /// 能不能对这张表做 VACUUM、ANALYZE:要是表的所有者、库的所有者(共享的系统表除外)或超级用户,
    /// PostgreSQL 17 起也可以是有 MAINTAIN 权限的用户。没有权限时数据库不报错,只发一条警告就跳过这张表,
    /// 所以要先问。
    pub(crate) async fn can_maintain(&self, table: &TableRef) -> CoreResult<bool> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let allowed: Option<bool> = sqlx::query_scalar(
            "SELECT (pg_has_role(d.datdba, 'USAGE') AND NOT c.relisshared)
                    OR CASE WHEN current_setting('server_version_num')::int >= 170000
                            THEN has_table_privilege(c.oid, 'MAINTAIN')
                            ELSE pg_has_role(c.relowner, 'USAGE')
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

    /// VACUUM 不能在事务块里执行:单独一条、走简单查询协议。等锁时间在这条连接上临时缩短,用完复原。
    pub(crate) async fn execute_outside_transaction(
        &self,
        database: &str,
        sql: &str,
    ) -> CoreResult<()> {
        let pool = self.pool(database).await?;
        let mut connection = pool.acquire().await?;
        let limit = format!("SET lock_timeout = '{LOCK_WAIT_SECONDS}s'");
        (&mut *connection).execute(limit.as_str()).await?;
        let done = (&mut *connection).execute(sql).await;
        if let Err(error) = (&mut *connection).execute("RESET lock_timeout").await {
            log::warn!("恢复 PostgreSQL 的等锁时间失败，这条连接不再放回连接池：{error}");
            connection.close_on_drop();
        }
        done.map_err(exec_error)?;
        Ok(())
    }

    /// 表连同索引、TOAST 的大小(字节)。分区表本身不存数据(量出来总是 0),要把各个分区加起来。
    pub(crate) async fn total_relation_size(&self, table: &TableRef) -> CoreResult<u64> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let size: Option<i64> = sqlx::query_scalar(
            "SELECT CASE WHEN c.relkind = 'p'
                         THEN (SELECT COALESCE(sum(pg_total_relation_size(p.relid)), 0)::bigint
                               FROM pg_partition_tree(c.oid::regclass) p)
                         ELSE pg_total_relation_size(c.oid)
                    END
             FROM pg_class c
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
