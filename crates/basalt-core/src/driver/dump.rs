//! 转储 SQL 文件要读的东西:建表语句、触发器、PostgreSQL 带序列的列。拼装见 [`crate::transfer::dump`]。
//!
//! 读的时候先把会影响写法的会话设置固定下来,和 mysqldump、pg_dump 一样:
//! - MySQL 在一条单独的连接上把时区设成 +00:00,TIMESTAMP 列的默认值按 UTC 写,和文件开头的 TIME_ZONE 对上;
//!   这条连接用完就断开,不放回连接池,免得时区设置留给别的操作。
//! - PostgreSQL 在一个只读事务里把 search_path 设成空,默认值里的序列、触发器调用的函数、自定义类型
//!   都写成带 schema 的名字,恢复到别的地方也不会绑错对象。

use sqlx::postgres::PgConnection;
use sqlx::{Connection, Executor, Row};

use super::mysql::MysqlSession;
use super::postgres::PgSession;
use super::postgres_design::read_design_on;
use super::sqlite::SqliteSession;
use super::table_info::show_create_with;
use crate::design::postgres_dump_statements;
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, quote_ident};
use crate::table::TableRef;
use crate::transfer::dump::{CreateScript, SequenceColumn, Trigger};

impl MysqlSession {
    pub(crate) async fn dump_script(&self, table: &TableRef) -> CoreResult<CreateScript> {
        let mut connection = self.pool.acquire().await?.detach();
        let result: CoreResult<CreateScript> = async {
            (&mut connection)
                .execute("SET time_zone = '+00:00'")
                .await?;
            let create = show_create_with(&mut connection, table).await?;
            let triggers = mysql_triggers(&mut connection, table).await?;
            Ok(CreateScript {
                table: vec![create],
                foreign_keys: Vec::new(),
                triggers,
            })
        }
        .await;
        if let Err(error) = connection.close().await {
            log::warn!("关闭转储用的连接失败：{error}");
        }
        result
    }
}

/// 表上的触发器,按执行顺序拼成 CREATE TRIGGER(不带 DEFINER,还原时归执行的用户;表名不带库名,和转储的建表语句一致),
/// 连同建它时的 sql_mode。
async fn mysql_triggers<'c, E>(executor: E, table: &TableRef) -> CoreResult<Vec<Trigger>>
where
    E: Executor<'c, Database = sqlx::MySql>,
{
    let rows = sqlx::query(
        "SELECT CAST(TRIGGER_NAME AS CHAR) AS name, CAST(ACTION_TIMING AS CHAR) AS timing,
                CAST(EVENT_MANIPULATION AS CHAR) AS event, CAST(ACTION_STATEMENT AS CHAR) AS body,
                CAST(SQL_MODE AS CHAR) AS sql_mode
         FROM information_schema.TRIGGERS
         WHERE EVENT_OBJECT_SCHEMA = ? AND EVENT_OBJECT_TABLE = ?
         ORDER BY ACTION_TIMING, EVENT_MANIPULATION, ACTION_ORDER",
    )
    .bind(&table.database)
    .bind(&table.table)
    .fetch_all(executor)
    .await?;
    let d = Dialect::Mysql;
    rows.iter()
        .map(|row| {
            let name: String = row.try_get("name")?;
            let timing: String = row.try_get("timing")?;
            let event: String = row.try_get("event")?;
            let body: String = row.try_get("body")?;
            Ok(Trigger {
                sql: format!(
                    "CREATE TRIGGER {} {timing} {event} ON {} FOR EACH ROW {}",
                    quote_ident(d, &name),
                    quote_ident(d, &table.table),
                    body.trim().trim_end_matches(';')
                ),
                sql_mode: row.try_get("sql_mode")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(CoreError::from)
}

impl PgSession {
    pub(crate) async fn dump_script(
        &self,
        table: &TableRef,
    ) -> CoreResult<(CreateScript, Vec<SequenceColumn>)> {
        let pool = self.pool(&table.database).await?;
        let mut transaction = pool.begin().await?;
        sqlx::query("SELECT set_config('search_path', '', true)")
            .execute(&mut *transaction)
            .await?;
        let design = read_design_on(&mut transaction, table).await?;
        let triggers = pg_triggers(&mut transaction, table).await?;
        let sequences = pg_sequence_columns(&mut transaction, table).await?;
        // 只读了东西,回滚就好。
        transaction.rollback().await?;
        let (create, foreign_keys) = postgres_dump_statements(&design);
        let script = CreateScript {
            table: create,
            foreign_keys,
            triggers: triggers.into_iter().map(Trigger::plain).collect(),
        };
        Ok((script, sequences))
    }
}

/// 表上用户建的触发器(不含外键约束自带的内部触发器)。
async fn pg_triggers(connection: &mut PgConnection, table: &TableRef) -> CoreResult<Vec<String>> {
    let schema = table.schema.as_deref().unwrap_or("public");
    let statements = sqlx::query_scalar(
        "SELECT pg_get_triggerdef(t.oid, true) FROM pg_trigger t
         JOIN pg_class c ON c.oid = t.tgrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2 AND NOT t.tgisinternal
         ORDER BY t.tgname",
    )
    .bind(schema)
    .bind(&table.table)
    .fetch_all(connection)
    .await?;
    Ok(statements)
}

/// 带序列的列(标识列、serial 列),转储时要重建序列、把它推到最大值。
async fn pg_sequence_columns(
    connection: &mut PgConnection,
    table: &TableRef,
) -> CoreResult<Vec<SequenceColumn>> {
    let schema = table.schema.as_deref().unwrap_or("public");
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
    .fetch_all(connection)
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

impl SqliteSession {
    /// `sqlite_master` 里的原文:表和索引在前,触发器单独给出(插完数据再建)。
    pub(crate) async fn dump_script(&self, table: &TableRef) -> CoreResult<CreateScript> {
        let (triggers, rest): (Vec<_>, Vec<_>) = self
            .own_objects(table)
            .await?
            .into_iter()
            .partition(|(kind, _)| kind == "trigger");
        Ok(CreateScript {
            table: rest.into_iter().map(|(_, sql)| sql).collect(),
            foreign_keys: Vec::new(),
            triggers: triggers
                .into_iter()
                .map(|(_, sql)| Trigger::plain(sql))
                .collect(),
        })
    }
}
