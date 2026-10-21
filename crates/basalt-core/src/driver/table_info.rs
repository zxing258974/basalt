//! 信息面板的查询:各家的表属性与建表语句。类型和拼装见 [`crate::table_info`]。
//! 数字、时间一律在 SQL 里转成文本再读:各家、各版本的整数宽度和有无符号都不一样。

use sqlx::Row;

use super::mysql::MysqlSession;
use super::postgres::PgSession;
use super::sqlite::SqliteSession;
use super::table_not_found;
use crate::design::postgres_create_statements;
use crate::error::CoreResult;
use crate::session::Session;
use crate::sql::bind::quote_string;
use crate::sql::{Dialect, quote_ident, table_name};
use crate::table::TableRef;
use crate::table_info::{Properties, TableDetails, join_statements};

/// SQLite 的信息面板最多数到这么多行。
pub(crate) const COUNT_LIMIT: i64 = 100_000;

fn yes_no(value: Option<String>) -> Option<String> {
    value.map(|v| match v.as_str() {
        "YES" | "1" => "是".to_owned(),
        "NO" | "0" => "否".to_owned(),
        _ => v,
    })
}

impl MysqlSession {
    pub(crate) async fn table_details(&self, table: &TableRef) -> CoreResult<TableDetails> {
        let row = sqlx::query(
            "SELECT CAST(TABLE_TYPE AS CHAR) AS kind, CAST(ENGINE AS CHAR) AS engine,
                    CAST(TABLE_ROWS AS CHAR) AS table_rows, CAST(DATA_LENGTH AS CHAR) AS data_length,
                    CAST(INDEX_LENGTH AS CHAR) AS index_length, CAST(DATA_FREE AS CHAR) AS data_free,
                    CAST(AVG_ROW_LENGTH AS CHAR) AS avg_row_length,
                    CAST(MAX_DATA_LENGTH AS CHAR) AS max_data_length,
                    CAST(AUTO_INCREMENT AS CHAR) AS auto_increment,
                    CAST(CREATE_TIME AS CHAR) AS create_time, CAST(UPDATE_TIME AS CHAR) AS update_time,
                    CAST(CHECK_TIME AS CHAR) AS check_time, CAST(TABLE_COLLATION AS CHAR) AS collation,
                    CAST(ROW_FORMAT AS CHAR) AS row_format, CAST(CREATE_OPTIONS AS CHAR) AS create_options,
                    CAST(TABLE_COMMENT AS CHAR) AS comment
             FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(&table.database)
        .bind(&table.table)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| table_not_found(&table.table))?;
        let get = |name: &str| row.try_get::<Option<String>, _>(name);
        let mut p = Properties::default();
        if get("kind")?.as_deref() == Some("VIEW") {
            p.text("类型", Some("视图".into()));
            let view = sqlx::query(
                "SELECT CAST(DEFINER AS CHAR) AS definer, CAST(SECURITY_TYPE AS CHAR) AS security,
                        CAST(IS_UPDATABLE AS CHAR) AS updatable, CAST(CHECK_OPTION AS CHAR) AS check_option
                 FROM information_schema.VIEWS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
            )
            .bind(&table.database)
            .bind(&table.table)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(view) = view {
                let get = |name: &str| view.try_get::<Option<String>, _>(name);
                p.text("定义者", get("definer")?);
                p.text("安全性", get("security")?);
                p.text("可更新", yes_no(get("updatable")?));
                p.text("检查选项", get("check_option")?.filter(|v| v != "NONE"));
            }
        } else {
            p.count("行（估算）", get("table_rows")?);
            p.bytes("数据长度", get("data_length")?);
            p.text("引擎", get("engine")?);
            p.datetime("创建日期", get("create_time")?);
            p.datetime("修改日期", get("update_time")?);
            p.text("排序规则", get("collation")?);
            p.text("行格式", get("row_format")?);
            p.bytes("平均行长度", get("avg_row_length")?);
            p.bytes("最大数据长度", get("max_data_length")?);
            p.bytes("索引长度", get("index_length")?);
            p.datetime("检查时间", get("check_time")?);
            p.count("自动递增", get("auto_increment")?);
            p.bytes("数据可用空间", get("data_free")?);
            p.text("创建选项", get("create_options")?);
            p.text("注释", get("comment")?);
        }
        Ok(TableDetails {
            properties: p.into_vec(),
            ddl: join_statements([self.show_create(table).await?]),
            ddl_generated: false,
        })
    }

    /// SHOW CREATE TABLE 的原文。
    pub(crate) async fn show_create(&self, table: &TableRef) -> CoreResult<String> {
        let show = sqlx::query(&format!(
            "SHOW CREATE TABLE {}",
            table_name(Dialect::Mysql, table)
        ))
        .persistent(false)
        .fetch_one(&self.pool)
        .await?;
        // 第二列是建表(或建视图)语句;二进制排序规则下会以字节返回。
        Ok(match show.try_get::<String, _>(1) {
            Ok(text) => text,
            Err(_) => String::from_utf8_lossy(&show.try_get::<Vec<u8>, _>(1)?).into_owned(),
        })
    }
}

impl PgSession {
    /// `session` 是这个连接本身:建表语句要借表结构设计读出来的结构生成。
    pub(crate) async fn table_details(
        &self,
        session: &Session,
        table: &TableRef,
    ) -> CoreResult<TableDetails> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let pool = self.pool(&table.database).await?;
        let row = sqlx::query(
            "SELECT c.relkind::text AS kind, c.reltuples::bigint::text AS estimate,
                    s.n_live_tup::text AS live, s.n_dead_tup::text AS dead,
                    pg_total_relation_size(c.oid)::text AS total, pg_relation_size(c.oid)::text AS data,
                    pg_indexes_size(c.oid)::text AS indexes, pg_get_userbyid(c.relowner)::text AS owner,
                    t.spcname::text AS tablespace, c.relpersistence::text AS persistence,
                    GREATEST(s.last_vacuum, s.last_autovacuum)::text AS vacuumed,
                    GREATEST(s.last_analyze, s.last_autoanalyze)::text AS analyzed,
                    obj_description(c.oid, 'pg_class') AS comment,
                    CASE WHEN c.relkind IN ('v', 'm') THEN pg_get_viewdef(c.oid, true) END AS view_def
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN pg_tablespace t ON t.oid = c.reltablespace
             LEFT JOIN pg_stat_all_tables s ON s.relid = c.oid
             WHERE n.nspname = $1 AND c.relname = $2",
        )
        .bind(schema)
        .bind(&table.table)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| table_not_found(&table.table))?;
        let get = |name: &str| row.try_get::<Option<String>, _>(name);
        let kind = get("kind")?.unwrap_or_default();
        let mut p = Properties::default();
        let label = match kind.as_str() {
            "v" => "视图",
            "m" => "物化视图",
            "p" => "分区表",
            "f" => "外部表",
            _ => "表",
        };
        p.text("类型", Some(label.into()));
        if kind != "v" {
            p.count("行（估算）", get("estimate")?);
            p.count("存活行", get("live")?);
            p.count("死行", get("dead")?);
            p.bytes("总大小", get("total")?);
            p.bytes("数据大小", get("data")?);
            p.bytes("索引大小", get("indexes")?);
        }
        p.text("所有者", get("owner")?);
        if kind != "v" {
            p.text(
                "表空间",
                Some(get("tablespace")?.unwrap_or_else(|| "pg_default".into())),
            );
            let persistence = match get("persistence")?.as_deref() {
                Some("u") => "不记日志（UNLOGGED）",
                Some("t") => "临时",
                _ => "普通",
            };
            p.text("持久性", Some(persistence.into()));
            p.datetime("最后清理", get("vacuumed")?);
            p.datetime("最后分析", get("analyzed")?);
        }
        p.text("注释", get("comment")?);

        let name = table_name(Dialect::Postgres, table);
        let (ddl, generated) = match (kind.as_str(), get("view_def")?) {
            ("v", Some(definition)) => (
                format!("CREATE VIEW {name} AS\n{}", definition.trim_end()),
                false,
            ),
            ("m", Some(definition)) => (
                format!(
                    "CREATE MATERIALIZED VIEW {name} AS\n{}",
                    definition.trim_end()
                ),
                false,
            ),
            _ => match session.read_design(table).await {
                Ok(design) => (join_statements(postgres_create_statements(&design)), true),
                Err(error) => (format!("-- 读不到表结构，生成不了建表语句：{error}"), true),
            },
        };
        Ok(TableDetails {
            properties: p.into_vec(),
            ddl: if ddl.starts_with("--") {
                ddl
            } else {
                join_statements([ddl])
            },
            ddl_generated: generated,
        })
    }
}

impl SqliteSession {
    /// `sqlite_master` 里和这张表有关的对象(类型、语句):先是表或视图本身,再是索引,最后是触发器。
    pub(crate) async fn own_objects(&self, table: &TableRef) -> CoreResult<Vec<(String, String)>> {
        let schema = quote_ident(Dialect::Sqlite, &table.database);
        let objects = sqlx::query(&format!(
            "SELECT type, sql FROM {schema}.sqlite_master
             WHERE tbl_name = ? AND sql IS NOT NULL
             ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 0 WHEN 'index' THEN 1 ELSE 2 END, name"
        ))
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| Ok((row.try_get("type")?, row.try_get("sql")?)))
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
        if objects.is_empty() {
            return Err(table_not_found(&table.table));
        }
        Ok(objects)
    }

    pub(crate) async fn table_details(&self, table: &TableRef) -> CoreResult<TableDetails> {
        let d = Dialect::Sqlite;
        let schema = quote_ident(d, &table.database);
        let objects = self.own_objects(table).await?;
        let is_view = objects[0].0 == "view";
        let mut p = Properties::default();
        p.text("类型", Some(if is_view { "视图" } else { "表" }.into()));
        if !is_view {
            // SQLite 没有行数统计,只能数;大表全表计数会卡住连接,最多数到 COUNT_LIMIT 行。
            let counted: i64 = sqlx::query_scalar(&format!(
                "SELECT count(*) FROM (SELECT 1 FROM {} LIMIT {})",
                table_name(d, table),
                COUNT_LIMIT + 1
            ))
            .fetch_one(&self.pool)
            .await?;
            if counted > COUNT_LIMIT {
                p.text("行", Some(format!("超过 {} 万", COUNT_LIMIT / 10_000)));
            } else {
                p.count("行", Some(counted.to_string()));
            }
            // table_list 要 SQLite 3.37;更老的版本读不到就不显示。
            let options = sqlx::query(&format!(
                "PRAGMA {schema}.table_list({})",
                quote_string(d, &table.table)
            ))
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
            if let Some(options) = options {
                let flag = |name: &str| options.try_get::<i64, _>(name).ok().map(|v| v.to_string());
                p.text("WITHOUT ROWID", yes_no(flag("wr")));
                p.text("STRICT", yes_no(flag("strict")));
            }
            // 只有用了 AUTOINCREMENT 的表才有 sqlite_sequence 里的记录。
            let sequence: Option<String> = sqlx::query_scalar(&format!(
                "SELECT CAST(seq AS TEXT) FROM {schema}.sqlite_sequence WHERE name = ?"
            ))
            .bind(&table.table)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
            p.count("自动递增", sequence);
            let indexes = objects.iter().filter(|(kind, _)| kind == "index").count();
            p.count("索引", Some(indexes.to_string()));
        }
        Ok(TableDetails {
            properties: p.into_vec(),
            ddl: join_statements(objects.into_iter().map(|(_, sql)| sql)),
            ddl_generated: false,
        })
    }
}
