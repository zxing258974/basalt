//! MySQL / MariaDB 的表数据:列信息、按列类型解码、在事务里执行修改。

use sqlx::mysql::{MySql, MySqlArguments, MySqlRow};
use sqlx::query::Query;
use sqlx::{Row, TypeInfo, ValueRef};

use super::mysql::MysqlSession;
use super::{check_affected, finish_transaction, statement_failed, table_not_found};
use crate::error::CoreResult;
use crate::sql::bind::BindValue;
use crate::sql::dml::Statement;
use crate::table::{ColumnInfo, TableRef, ValueType};
use crate::value::{CellValue, widen_f32};

impl MysqlSession {
    pub async fn columns(&self, table: &TableRef) -> CoreResult<Vec<ColumnInfo>> {
        // information_schema 的列在 MySQL 8 里可能是二进制排序规则,统一 CAST 成字符。
        let rows = sqlx::query(
            "SELECT CAST(COLUMN_NAME AS CHAR) AS name,
                    CAST(DATA_TYPE AS CHAR) AS data_type,
                    CAST(COLUMN_TYPE AS CHAR) AS column_type,
                    CAST(IS_NULLABLE AS CHAR) AS is_nullable,
                    CAST(COLUMN_KEY AS CHAR) AS column_key,
                    CAST(COLUMN_DEFAULT AS CHAR) AS column_default,
                    CAST(EXTRA AS CHAR) AS extra
             FROM information_schema.COLUMNS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY ORDINAL_POSITION",
        )
        .bind(&table.database)
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Err(table_not_found(&format!(
                "{}.{}",
                table.database, table.table
            )));
        }
        rows.iter()
            .map(|row| {
                let data_type: String = row.try_get("data_type")?;
                let column_type: String = row.try_get("column_type")?;
                let extra = row
                    .try_get::<Option<String>, _>("extra")?
                    .unwrap_or_default();
                let default: Option<String> = row.try_get("column_default")?;
                Ok(ColumnInfo {
                    value_type: value_type(&data_type, &column_type),
                    // 没有主键时,不含 NULL 的唯一索引也会显示成 PRI,一样能定位行。
                    is_primary_key: row.try_get::<String, _>("column_key")? == "PRI",
                    nullable: row.try_get::<String, _>("is_nullable")? == "YES",
                    has_default: default.is_some() || extra.contains("auto_increment"),
                    is_generated: is_generated(&extra),
                    ..ColumnInfo::new(&row.try_get::<String, _>("name")?, &column_type)
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// 表的存储引擎;视图为 `None`。
    pub async fn table_engine(&self, table: &TableRef) -> CoreResult<Option<String>> {
        let engine: Option<Option<String>> = sqlx::query_scalar(
            "SELECT CAST(ENGINE AS CHAR) FROM information_schema.TABLES
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(&table.database)
        .bind(&table.table)
        .fetch_optional(&self.pool)
        .await?;
        Ok(engine.flatten())
    }

    pub async fn fetch_rows(
        &self,
        sql: &str,
        columns: &[ColumnInfo],
    ) -> CoreResult<Vec<Vec<CellValue>>> {
        let rows = sqlx::query(sql)
            .persistent(false)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| decode_cell(row, index, column.value_type))
                    .collect()
            })
            .collect())
    }

    pub async fn execute(&self, statements: &[Statement]) -> CoreResult<u64> {
        let mut transaction = self.pool.begin().await?;
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
    query: Query<'_, MySql, MySqlArguments>,
    value: BindValue,
) -> Query<'_, MySql, MySqlArguments> {
    match value {
        BindValue::Null => query.bind(None::<String>),
        BindValue::Bool(b) => query.bind(b),
        BindValue::Int(n) => query.bind(n),
        BindValue::UInt(n) => query.bind(n),
        BindValue::Float(f) => query.bind(f),
        BindValue::Decimal(d) => query.bind(d),
        BindValue::Text(text) => query.bind(text),
        BindValue::Bytes(bytes) => query.bind(bytes),
    }
}

fn value_type(data_type: &str, column_type: &str) -> ValueType {
    match data_type.to_ascii_lowercase().as_str() {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" => ValueType::Integer {
            unsigned: column_type.to_ascii_lowercase().contains("unsigned"),
        },
        "float" | "double" | "real" => ValueType::Float,
        "decimal" | "numeric" => ValueType::Decimal,
        "bit" => ValueType::Bit,
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => {
            ValueType::Binary
        }
        _ => ValueType::Text,
    }
}

/// MySQL 8 的表达式默认值在 EXTRA 里是 DEFAULT_GENERATED,那不是生成列。
fn is_generated(extra: &str) -> bool {
    [
        "VIRTUAL GENERATED",
        "STORED GENERATED",
        "PERSISTENT GENERATED",
    ]
    .iter()
    .any(|marker| extra.to_ascii_uppercase().contains(marker))
}

/// 按结果列的线上类型解码一个单元格。`declared` 是表结构里的类型:
/// utf8mb4_bin 这类二进制排序规则的文本列在线上被报成 VARBINARY / BLOB,
/// 要靠它认回文本。解不了或解码失败的值返回 [`CellValue::Unsupported`]。
pub(crate) fn decode_cell(row: &MySqlRow, index: usize, declared: ValueType) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::unsupported("?");
    };
    let type_name = raw.type_info().name().to_owned();
    // sqlx 把零日期 0000-00-00 也报成 NULL;日期类型改看原始字节,真 NULL 才读不到字节。
    if matches!(type_name.as_str(), "DATE" | "DATETIME" | "TIMESTAMP") {
        return match raw_bytes(row, index) {
            Ok(bytes) if type_name == "DATE" => {
                flatten(Ok(format_date(&bytes).map(CellValue::Date)), type_name)
            }
            Ok(bytes) => flatten(
                Ok(format_datetime(&bytes).map(CellValue::DateTime)),
                type_name,
            ),
            Err(_) => CellValue::Null,
        };
    }
    if raw.is_null() {
        return CellValue::Null;
    }
    let decoded = match type_name.as_str() {
        "BOOLEAN" | "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => row
            .try_get_unchecked::<i64, _>(index)
            .map(|n| Some(CellValue::Int(n))),
        name if name.ends_with(" UNSIGNED") => row
            .try_get_unchecked::<u64, _>(index)
            .map(|n| Some(CellValue::UInt(n))),
        "YEAR" | "BIT" => row
            .try_get_unchecked::<u64, _>(index)
            .map(|n| Some(CellValue::UInt(n))),
        "FLOAT" => row
            .try_get_unchecked::<f32, _>(index)
            .map(|f| Some(CellValue::Float(widen_f32(f)))),
        "DOUBLE" => row
            .try_get_unchecked::<f64, _>(index)
            .map(|f| Some(CellValue::Float(f))),
        "DECIMAL" => row
            .try_get_unchecked::<String, _>(index)
            .map(|s| Some(CellValue::Decimal(s))),
        "DATE" => raw_bytes(row, index).map(|b| format_date(&b).map(CellValue::Date)),
        "DATETIME" | "TIMESTAMP" => {
            raw_bytes(row, index).map(|b| format_datetime(&b).map(CellValue::DateTime))
        }
        "TIME" => raw_bytes(row, index).map(|b| format_time(&b).map(CellValue::Time)),
        "JSON" => row
            .try_get_unchecked::<String, _>(index)
            .map(|s| Some(CellValue::Json(s))),
        "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" | "SET" => {
            row.try_get_unchecked::<String, _>(index)
                .map(|s| Some(CellValue::Text(s)))
        }
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => {
            raw_bytes(row, index).map(|bytes| Some(binary_cell(bytes, declared)))
        }
        _ => return CellValue::Unsupported { type_name },
    };
    flatten(decoded, type_name)
}

fn raw_bytes(row: &MySqlRow, index: usize) -> Result<Vec<u8>, sqlx::Error> {
    row.try_get_unchecked::<Vec<u8>, _>(index)
}

fn binary_cell(bytes: Vec<u8>, declared: ValueType) -> CellValue {
    if declared == ValueType::Text {
        String::from_utf8(bytes).map_or_else(|e| CellValue::Bytes(e.into_bytes()), CellValue::Text)
    } else {
        CellValue::Bytes(bytes)
    }
}

/// 解码失败也不能静默丢掉:记日志并显示成类型名。
fn flatten(decoded: Result<Option<CellValue>, sqlx::Error>, type_name: String) -> CellValue {
    match decoded {
        Ok(Some(value)) => value,
        Ok(None) => CellValue::Unsupported { type_name },
        Err(error) => {
            log::warn!("解码 MySQL {type_name} 失败：{error}");
            CellValue::Unsupported { type_name }
        }
    }
}

// 二进制协议里日期时间的线上格式:首字节是长度,全零时长度为 0。
// 不经过 chrono:零日期 0000-00-00、ALLOW_INVALID_DATES 下的 2026-02-30 都要原样显示。

fn format_date(raw: &[u8]) -> Option<String> {
    let (date, _) = split_date(raw)?;
    Some(date)
}

fn format_datetime(raw: &[u8]) -> Option<String> {
    let (date, rest) = split_date(raw)?;
    let time = match rest {
        [] => "00:00:00".to_owned(),
        [h, m, s, micros @ ..] => clock(u32::from(*h), *m, *s, micros)?,
        _ => return None,
    };
    Some(format!("{date} {time}"))
}

fn split_date(raw: &[u8]) -> Option<(String, &[u8])> {
    let (&len, body) = raw.split_first()?;
    if len == 0 {
        return Some(("0000-00-00".into(), &[]));
    }
    let body = body.get(..usize::from(len))?;
    let [y0, y1, month, day, rest @ ..] = body else {
        return None;
    };
    let year = u16::from_le_bytes([*y0, *y1]);
    Some((format!("{year:04}-{month:02}-{day:02}"), rest))
}

/// TIME 可以是负数,也可以超过 24 小时(-838:59:59 到 838:59:59)。
fn format_time(raw: &[u8]) -> Option<String> {
    let (&len, body) = raw.split_first()?;
    if len == 0 {
        return Some("00:00:00".into());
    }
    let [negative, d0, d1, d2, d3, h, m, s, micros @ ..] = body.get(..usize::from(len))? else {
        return None;
    };
    let days = u32::from_le_bytes([*d0, *d1, *d2, *d3]);
    let hours = days.checked_mul(24)?.checked_add(u32::from(*h))?;
    let sign = if *negative == 1 { "-" } else { "" };
    Some(format!("{sign}{}", clock(hours, *m, *s, micros)?))
}

fn clock(hours: u32, minutes: u8, seconds: u8, micros: &[u8]) -> Option<String> {
    let fraction = match micros {
        [] => 0,
        [a, b, c, d] => u32::from_le_bytes([*a, *b, *c, *d]),
        _ => return None,
    };
    let mut out = format!("{hours:02}:{minutes:02}:{seconds:02}");
    if fraction != 0 {
        out.push_str(format!(".{fraction:06}").trim_end_matches('0'));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_partial_dates_are_shown_as_stored() {
        assert_eq!(format_date(&[0]).as_deref(), Some("0000-00-00"));
        assert_eq!(
            format_date(&[4, 0xEA, 0x07, 10, 20]).as_deref(),
            Some("2026-10-20")
        );
        assert_eq!(
            format_date(&[4, 0xEA, 0x07, 2, 30]).as_deref(),
            Some("2026-02-30")
        );
        assert_eq!(
            format_datetime(&[0]).as_deref(),
            Some("0000-00-00 00:00:00")
        );
        assert_eq!(
            format_datetime(&[4, 0xEA, 0x07, 10, 20]).as_deref(),
            Some("2026-10-20 00:00:00")
        );
        assert_eq!(
            format_datetime(&[7, 0xEA, 0x07, 10, 20, 8, 5, 9]).as_deref(),
            Some("2026-10-20 08:05:09")
        );
        // 120000 微秒 = .12
        assert_eq!(
            format_datetime(&[11, 0xEA, 0x07, 10, 20, 8, 5, 9, 0xC0, 0xD4, 0x01, 0x00]).as_deref(),
            Some("2026-10-20 08:05:09.12")
        );
        assert_eq!(format_date(&[4, 1]), None, "长度不够");
    }

    #[test]
    fn times_can_be_negative_and_exceed_a_day() {
        assert_eq!(format_time(&[0]).as_deref(), Some("00:00:00"));
        assert_eq!(
            format_time(&[8, 0, 0, 0, 0, 0, 13, 5, 1]).as_deref(),
            Some("13:05:01")
        );
        // -34 天 22 小时 59 分 59 秒 = -838:59:59
        assert_eq!(
            format_time(&[8, 1, 34, 0, 0, 0, 22, 59, 59]).as_deref(),
            Some("-838:59:59")
        );
        assert_eq!(
            format_time(&[12, 0, 0, 0, 0, 0, 1, 2, 3, 1, 0, 0, 0]).as_deref(),
            Some("01:02:03.000001")
        );
    }

    #[test]
    fn declared_types_drive_write_conversion() {
        assert_eq!(
            value_type("int", "int(10) unsigned"),
            ValueType::Integer { unsigned: true }
        );
        assert_eq!(
            value_type("tinyint", "tinyint(1)"),
            ValueType::Integer { unsigned: false }
        );
        assert_eq!(value_type("decimal", "decimal(10,2)"), ValueType::Decimal);
        assert_eq!(value_type("varbinary", "varbinary(16)"), ValueType::Binary);
        assert_eq!(value_type("json", "json"), ValueType::Text);
        assert!(is_generated("STORED GENERATED"));
        assert!(!is_generated(
            "DEFAULT_GENERATED on update CURRENT_TIMESTAMP"
        ));
    }

    #[test]
    fn binary_wire_values_of_text_columns_become_text() {
        assert_eq!(
            binary_cell(b"abc".to_vec(), ValueType::Text),
            CellValue::Text("abc".into())
        );
        assert_eq!(
            binary_cell(vec![0xff], ValueType::Text),
            CellValue::Bytes(vec![0xff])
        );
        assert_eq!(
            binary_cell(b"abc".to_vec(), ValueType::Binary),
            CellValue::Bytes(b"abc".to_vec())
        );
    }
}
