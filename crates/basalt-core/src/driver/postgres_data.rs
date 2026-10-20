//! PostgreSQL 的表数据:列信息、按列类型解码、在事务里执行修改。
//!
//! 下面 [`NATIVE_TYPES`] 里的类型按二进制格式直接解码;其余类型(数组、区间、
//! inet、money、枚举、几何……)在 SELECT 里转成 text 读出,写回时
//! `$1::原类型` 交给数据库解析,所以都能看也能改。

use chrono::{Local, Offset, TimeZone};
use sqlx::postgres::{PgArguments, PgRow, PgValueFormat, PgValueRef, Postgres};
use sqlx::query::Query;
use sqlx::{Row, TypeInfo, ValueRef};

use super::pg_binary::{
    date_to_string, numeric_to_string, time_to_string, timestamp_to_string, timestamp_to_utc,
};
use super::postgres::PgSession;
use super::{check_affected, finish_transaction, statement_failed, table_not_found};
use crate::error::{CoreError, CoreResult};
use crate::sql::bind::BindValue;
use crate::sql::dml::Statement;
use crate::table::{ColumnInfo, TableRef, ValueType};
use crate::value::{CellValue, widen_f32};

/// 能直接解码的类型(pg_type.typname;域取其基础类型,线上报的也是基础类型)。
const NATIVE_TYPES: &[&str] = &[
    "bool",
    "int2",
    "int4",
    "int8",
    "oid",
    "float4",
    "float8",
    "numeric",
    "text",
    "varchar",
    "bpchar",
    "name",
    "char",
    "uuid",
    "bytea",
    "json",
    "jsonb",
    "date",
    "time",
    "timestamp",
    "timestamptz",
];

/// 文本参数可以直接赋给这两种列,不必写转换。
const TEXT_TYPES: &[&str] = &["text", "character varying"];

// format_type(oid, -1) 给出不带长度的类型名:char 列得到 bpchar 而不是
// character(=character(1)),转换时不会被截断,长度仍由列定义检查。
const COLUMNS_SQL: &str = "
    SELECT a.attname::text AS name,
           format_type(a.atttypid, a.atttypmod) AS data_type,
           format_type(a.atttypid, -1) AS cast_type,
           (CASE WHEN t.typtype = 'd' THEN bt.typname ELSE t.typname END)::text AS base_type,
           NOT a.attnotnull AS nullable,
           (a.atthasdef OR a.attidentity <> '') AS has_default,
           (a.attgenerated <> '') AS is_generated,
           COALESCE(a.attnum = ANY (i.indkey), false) AS is_primary_key
    FROM pg_attribute a
    JOIN pg_class c ON c.oid = a.attrelid
    JOIN pg_namespace n ON n.oid = c.relnamespace
    JOIN pg_type t ON t.oid = a.atttypid
    LEFT JOIN pg_type bt ON bt.oid = t.typbasetype
    LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary
    WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
    ORDER BY a.attnum";

fn schema_of(table: &TableRef) -> CoreResult<&str> {
    table
        .schema
        .as_deref()
        .ok_or_else(|| CoreError::Unsupported("PostgreSQL 需要指定 schema".into()))
}

impl PgSession {
    pub async fn columns(&self, table: &TableRef) -> CoreResult<Vec<ColumnInfo>> {
        let schema = schema_of(table)?;
        let pool = self.pool(&table.database).await?;
        let rows = sqlx::query(COLUMNS_SQL)
            .bind(schema)
            .bind(&table.table)
            .fetch_all(&pool)
            .await?;
        if rows.is_empty() {
            return Err(table_not_found(&format!("{schema}.{}", table.table)));
        }
        rows.iter()
            .map(|row| {
                let base_type: String = row.try_get("base_type")?;
                let cast_type: String = row.try_get("cast_type")?;
                Ok(ColumnInfo {
                    is_primary_key: row.try_get("is_primary_key")?,
                    nullable: row.try_get("nullable")?,
                    has_default: row.try_get("has_default")?,
                    is_generated: row.try_get("is_generated")?,
                    value_type: if base_type == "bytea" {
                        ValueType::Binary
                    } else {
                        ValueType::Text
                    },
                    select_as_text: !NATIVE_TYPES.contains(&base_type.as_str()),
                    write_cast: (!TEXT_TYPES.contains(&cast_type.as_str())).then_some(cast_type),
                    ..ColumnInfo::new(
                        &row.try_get::<String, _>("name")?,
                        &row.try_get::<String, _>("data_type")?,
                    )
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    pub async fn fetch_rows(
        &self,
        database: &str,
        sql: &str,
        columns: &[ColumnInfo],
    ) -> CoreResult<Vec<Vec<CellValue>>> {
        let pool = self.pool(database).await?;
        let rows = sqlx::query(sql).persistent(false).fetch_all(&pool).await?;
        Ok(rows
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|index| decode_cell(row, index))
                    .collect()
            })
            .collect())
    }

    pub async fn execute(&self, database: &str, statements: &[Statement]) -> CoreResult<u64> {
        let pool = self.pool(database).await?;
        let mut transaction = pool.begin().await?;
        let result = async {
            let mut affected = 0;
            for (index, statement) in statements.iter().enumerate() {
                let sql = statement.sql();
                let query = statement
                    .params()
                    .into_iter()
                    .fold(sqlx::query(&sql).persistent(false), bind);
                let done = query
                    .execute(&mut *transaction)
                    .await
                    .map_err(|error| statement_failed(index, error))?;
                check_affected(index, done.rows_affected())?;
                affected += done.rows_affected();
            }
            Ok(affected)
        }
        .await;
        finish_transaction(transaction, result).await
    }
}

fn bind(
    query: Query<'_, Postgres, PgArguments>,
    value: BindValue,
) -> Query<'_, Postgres, PgArguments> {
    match value {
        BindValue::Null => query.bind(None::<String>),
        BindValue::Bool(b) => query.bind(b),
        BindValue::Int(n) => query.bind(n),
        BindValue::UInt(n) => query.bind(n.to_string()),
        BindValue::Float(f) => query.bind(f),
        BindValue::Decimal(d) => query.bind(d),
        BindValue::Text(text) => query.bind(text),
        BindValue::Bytes(bytes) => query.bind(bytes),
    }
}

/// 按结果列的线上类型解码一个单元格。解不了或解码失败的值返回
/// [`CellValue::Unsupported`],不静默丢弃。
pub(crate) fn decode_cell(row: &PgRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::unsupported("?");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_owned();
    // 预备语句一律是二进制格式;万一拿到文本格式,原文就是 PostgreSQL 的写法。
    if raw.format() == PgValueFormat::Text {
        return raw.as_str().map_or_else(
            |_| CellValue::Unsupported { type_name },
            |s| CellValue::Text(s.to_owned()),
        );
    }
    match decode_binary(&raw, &type_name) {
        Some(value) => value,
        None => CellValue::Unsupported { type_name },
    }
}

fn decode_binary(raw: &PgValueRef<'_>, type_name: &str) -> Option<CellValue> {
    let bytes = raw.as_bytes().ok()?;
    Some(match type_name {
        "BOOL" => CellValue::Bool(*bytes.first()? != 0),
        "INT2" => CellValue::Int(i16::from_be_bytes(fixed(bytes)?).into()),
        "INT4" => CellValue::Int(i32::from_be_bytes(fixed(bytes)?).into()),
        "INT8" => CellValue::Int(i64::from_be_bytes(fixed(bytes)?)),
        "OID" => CellValue::UInt(u32::from_be_bytes(fixed(bytes)?).into()),
        "FLOAT4" => CellValue::Float(widen_f32(f32::from_be_bytes(fixed(bytes)?))),
        "FLOAT8" => CellValue::Float(f64::from_be_bytes(fixed(bytes)?)),
        "NUMERIC" => CellValue::Decimal(numeric_to_string(bytes)?),
        "TEXT" | "VARCHAR" | "CHAR" | "NAME" | "UNKNOWN" => CellValue::Text(utf8(bytes)?),
        // 单字节的 "char" 类型。
        "\"CHAR\"" => CellValue::Text(char::from(*bytes.first()?).to_string()),
        "UUID" => CellValue::Text(uuid::Uuid::from_bytes(fixed(bytes)?).to_string()),
        "BYTEA" => CellValue::Bytes(bytes.to_vec()),
        "JSON" => CellValue::Json(utf8(bytes)?),
        // JSONB 的二进制格式是版本号 1 加 JSON 文本。
        "JSONB" => match bytes.split_first()? {
            (1, text) => CellValue::Json(utf8(text)?),
            _ => return None,
        },
        "DATE" => CellValue::Date(date_to_string(i32::from_be_bytes(fixed(bytes)?))?),
        "TIME" => CellValue::Time(time_to_string(i64::from_be_bytes(fixed(bytes)?))),
        "TIMESTAMP" => CellValue::DateTime(timestamp_to_string(
            i64::from_be_bytes(fixed(bytes)?),
            None,
        )?),
        "TIMESTAMPTZ" => {
            let micros = i64::from_be_bytes(fixed(bytes)?);
            CellValue::DateTime(timestamp_to_string(micros, Some(local_offset(micros)))?)
        }
        _ => return None,
    })
}

/// 带时区的时间按本机时区显示。会话的 TimeZone 也设成了本机时区
/// (见 `postgres::connect_pool`),筛选条件里的时间字面量与显示一致。
fn local_offset(micros: i64) -> i32 {
    timestamp_to_utc(micros)
        .map(|utc| Local.offset_from_utc_datetime(&utc).fix().local_minus_utc())
        .unwrap_or(0)
}

fn fixed<const N: usize>(bytes: &[u8]) -> Option<[u8; N]> {
    bytes.try_into().ok()
}

fn utf8(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}
