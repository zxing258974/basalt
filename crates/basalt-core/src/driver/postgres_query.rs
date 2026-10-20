//! PostgreSQL 查询标签页的连接:用简单查询协议执行用户的 SQL。
//!
//! 简单查询协议的结果都是文本格式,数组、区间、枚举这些驱动不认识的类型也能原样显示;
//! 按结果列的类型转成 [`CellValue`]。一个连接只能连一个库,换库得换连接。

use sqlx::postgres::{PgConnection, PgPool, PgRow, PgValueFormat};
use sqlx::{Connection, Executor, Row, TypeInfo, ValueRef};

use super::postgres::PgSession;
use super::postgres_data;
use crate::error::{CoreError, CoreResult};
use crate::query::ResultColumn;
use crate::query::collect::{Collected, collect, described_columns};
use crate::sql::{Dialect, quote_ident};
use crate::value::{CellValue, from_hex};

pub(crate) struct PgEditor {
    pub(super) conn: PgConnection,
    pid: i32,
    database: String,
}

impl PgSession {
    /// 从这个库的池里取一条连接并脱离连接池,只归这个查询标签页用。`None` 是连接的默认库。
    pub(crate) async fn open_editor(&self, database: Option<&str>) -> CoreResult<PgEditor> {
        let database = database.unwrap_or(self.default_database()).to_owned();
        let mut conn = self.pool(&database).await?.acquire().await?.detach();
        let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut conn)
            .await?;
        Ok(PgEditor {
            conn,
            pid,
            database,
        })
    }

    /// 发 pg_cancel_backend 用的连接池(哪个库都行,取消请求是整个实例范围的)。
    pub(crate) async fn cancel_pool(&self) -> CoreResult<PgPool> {
        self.pool(self.default_database()).await
    }
}

impl PgEditor {
    pub(crate) fn pid(&self) -> i32 {
        self.pid
    }

    pub(crate) fn database(&self) -> &str {
        &self.database
    }

    /// 选了 schema 就把它放在 search_path 最前面;public 留在后面,装在 public 里的扩展函数照常可用。
    pub(crate) async fn set_schema(&mut self, schema: &str) -> CoreResult<()> {
        sqlx::query("SELECT set_config('search_path', $1, false)")
            .bind(search_path(schema))
            .execute(&mut self.conn)
            .await?;
        Ok(())
    }

    pub(crate) async fn current_schema(&mut self) -> CoreResult<Option<String>> {
        Ok(sqlx::query_scalar("SELECT current_schema()::text")
            .fetch_one(&mut self.conn)
            .await?)
    }

    pub(crate) async fn run(&mut self, sql: &str) -> Collected {
        let stream = sqlx::raw_sql(sql).fetch_many(&mut self.conn);
        collect(
            stream,
            decode_text_cell,
            sqlx::postgres::PgQueryResult::rows_affected,
        )
        .await
    }

    pub(crate) async fn describe(&mut self, sql: &str) -> Vec<ResultColumn> {
        described_columns((&mut self.conn).describe(sql).await)
    }

    pub(crate) async fn ping(&mut self) -> bool {
        self.conn.ping().await.is_ok()
    }

    pub(crate) async fn clear_cached_statements(&mut self) -> Result<(), sqlx::Error> {
        self.conn.clear_cached_statements().await
    }

    pub(crate) async fn close(self) {
        if let Err(error) = self.conn.close().await {
            log::warn!("关闭查询标签页的 PostgreSQL 连接失败：{error}");
        }
    }
}

pub(crate) fn search_path(schema: &str) -> String {
    let first = quote_ident(Dialect::Postgres, schema);
    if schema == "public" {
        first
    } else {
        format!("{first}, public")
    }
}

/// 在另一条连接上取消这个后端进程正在执行的语句。没有权限时数据库返回 false。
pub(crate) async fn cancel_backend(pool: &PgPool, pid: i32) -> CoreResult<()> {
    let accepted: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
        .bind(pid)
        .fetch_one(pool)
        .await?;
    if accepted {
        Ok(())
    } else {
        Err(CoreError::Database(
            "数据库没有接受取消请求，可能是当前用户没有权限取消这条语句。".into(),
        ))
    }
}

pub(crate) fn decode_text_cell(row: &PgRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::unsupported("?");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    if raw.format() == PgValueFormat::Binary {
        return postgres_data::decode_cell(row, index);
    }
    let type_name = raw.type_info().name().to_owned();
    match raw.as_str() {
        Ok(text) => text_value(&type_name, text),
        Err(_) => CellValue::Unsupported { type_name },
    }
}

/// 文本格式的一个非 NULL 值。数字解析失败(不该发生)时保留原文。
fn text_value(type_name: &str, text: &str) -> CellValue {
    let keep = || CellValue::Text(text.to_owned());
    match type_name {
        "BOOL" => match text {
            "t" => CellValue::Bool(true),
            "f" => CellValue::Bool(false),
            _ => keep(),
        },
        "INT2" | "INT4" | "INT8" => text.parse().map_or_else(|_| keep(), CellValue::Int),
        "OID" => text.parse().map_or_else(|_| keep(), CellValue::UInt),
        // Rust 认得 PostgreSQL 写的 Infinity、-Infinity、NaN。
        "FLOAT4" | "FLOAT8" => text.parse().map_or_else(|_| keep(), CellValue::Float),
        "NUMERIC" => CellValue::Decimal(text.to_owned()),
        "DATE" => CellValue::Date(text.to_owned()),
        "TIME" | "TIMETZ" => CellValue::Time(text.to_owned()),
        "TIMESTAMP" | "TIMESTAMPTZ" => CellValue::DateTime(text.to_owned()),
        "JSON" | "JSONB" => CellValue::Json(text.to_owned()),
        // bytea_output = hex(默认)时是 \x 加十六进制;escape 格式就照原文显示。
        "BYTEA" => text
            .strip_prefix("\\x")
            .and_then(from_hex)
            .map_or_else(keep, CellValue::Bytes),
        _ => keep(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_follow_postgres_text_output() {
        assert_eq!(text_value("BOOL", "t"), CellValue::Bool(true));
        assert_eq!(text_value("BOOL", "f"), CellValue::Bool(false));
        assert_eq!(text_value("INT8", "-42"), CellValue::Int(-42));
        assert_eq!(text_value("OID", "16384"), CellValue::UInt(16384));
        assert_eq!(
            text_value("FLOAT8", "Infinity"),
            CellValue::Float(f64::INFINITY)
        );
        assert!(matches!(text_value("FLOAT4", "NaN"), CellValue::Float(f) if f.is_nan()));
        assert_eq!(
            text_value("NUMERIC", "3.14159265358979323846264338327950288"),
            CellValue::Decimal("3.14159265358979323846264338327950288".into())
        );
    }

    #[test]
    fn temporal_and_json_values_are_kept_verbatim() {
        assert_eq!(
            text_value("DATE", "infinity"),
            CellValue::Date("infinity".into())
        );
        assert_eq!(
            text_value("TIMESTAMPTZ", "2026-10-20 15:07:44+08"),
            CellValue::DateTime("2026-10-20 15:07:44+08".into())
        );
        assert_eq!(
            text_value("TIMETZ", "10:00:00+08"),
            CellValue::Time("10:00:00+08".into())
        );
        assert_eq!(
            text_value("JSONB", "{\"a\": 1}"),
            CellValue::Json("{\"a\": 1}".into())
        );
    }

    #[test]
    fn bytea_hex_becomes_bytes_and_other_types_text() {
        assert_eq!(
            text_value("BYTEA", "\\x00ff"),
            CellValue::Bytes(vec![0x00, 0xff])
        );
        assert_eq!(text_value("BYTEA", "abc"), CellValue::Text("abc".into()));
        assert_eq!(
            text_value("INT4[]", "{1,2}"),
            CellValue::Text("{1,2}".into())
        );
        assert_eq!(text_value("?", "happy"), CellValue::Text("happy".into()));
        assert_eq!(text_value("INT4", "x"), CellValue::Text("x".into()));
    }

    #[test]
    fn search_path_keeps_public_after_the_chosen_schema() {
        assert_eq!(search_path("public"), "\"public\"");
        assert_eq!(search_path("diku_mod_users"), "\"diku_mod_users\", public");
        assert_eq!(search_path("we\"ird"), "\"we\"\"ird\", public");
    }
}
