//! MySQL / MariaDB 的表数据:列信息、按列类型解码、在事务里执行修改。

use sqlx::mysql::{MySql, MySqlArguments, MySqlRow};
use sqlx::query::Query;
use sqlx::{Executor, Row, TypeInfo, ValueRef};

use super::mysql::MysqlSession;
use super::mysql_query::text_value;
use super::{check_affected, finish_transaction, statement_failed, table_not_found};
use crate::error::CoreResult;
use crate::sql::bind::BindValue;
use crate::sql::dml::Statement;
use crate::sql::dynamic;
use crate::table::{ColumnInfo, TableRef, ValueType};
use crate::value::CellValue;

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
        // 走文本协议:预备语句读视图,在打开的表比 table_definition_cache 多的服务器上次次报 1615
        // (Prepared statement needs to be re-prepared)。SQL 拼好了不带参数,筛选条件已查过不含分号。
        let rows = (&self.pool).fetch_all(dynamic(sql)).await?;
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
                    .fold(sqlx::query(dynamic(&sql)).persistent(false), bind);
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

pub(super) fn bind(
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

/// 表数据页、导出整张表时线上类型能显示的列;别的(GEOMETRY 等)显示成类型名,不让改。
const SHOWN_TYPES: [&str; 24] = [
    "BOOLEAN",
    "TINYINT",
    "SMALLINT",
    "MEDIUMINT",
    "INT",
    "BIGINT",
    "YEAR",
    "BIT",
    "FLOAT",
    "DOUBLE",
    "DECIMAL",
    "DATE",
    "DATETIME",
    "TIMESTAMP",
    "TIME",
    "JSON",
    "CHAR",
    "VARCHAR",
    "TINYTEXT",
    "TEXT",
    "MEDIUMTEXT",
    "LONGTEXT",
    "ENUM",
    "SET",
];
const BINARY_TYPES: [&str; 6] = [
    "BINARY",
    "VARBINARY",
    "TINYBLOB",
    "BLOB",
    "MEDIUMBLOB",
    "LONGBLOB",
];

/// 文本协议里的一个单元格,按结果列的线上类型解码。`declared` 是表结构里的类型:
/// utf8mb4_bin 这类二进制排序规则的文本列在线上被报成 VARBINARY / BLOB,要靠它认回文本。
/// 日期时间照服务器发来的原文,零日期 0000-00-00 也一样。解不了的值返回 [`CellValue::Unsupported`]。
pub(crate) fn decode_cell(row: &MySqlRow, index: usize, declared: ValueType) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::unsupported("?");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_owned();
    match row.try_get_unchecked::<Vec<u8>, _>(index) {
        Ok(bytes) => table_cell(type_name, bytes, declared),
        Err(error) => {
            log::warn!("读取 MySQL {type_name} 失败：{error}");
            CellValue::Unsupported { type_name }
        }
    }
}

/// 文本协议里的一个非 NULL 值:二进制类型看表结构,能显示的交给和 SQL 编辑器同一套解码。
fn table_cell(type_name: String, bytes: Vec<u8>, declared: ValueType) -> CellValue {
    let name = type_name.as_str();
    if BINARY_TYPES.contains(&name) {
        binary_cell(bytes, declared)
    } else if SHOWN_TYPES.contains(&name) || name.ends_with(" UNSIGNED") {
        text_value(name, bytes)
    } else {
        CellValue::Unsupported { type_name }
    }
}

fn binary_cell(bytes: Vec<u8>, declared: ValueType) -> CellValue {
    if declared == ValueType::Text {
        String::from_utf8(bytes).map_or_else(|e| CellValue::Bytes(e.into_bytes()), CellValue::Text)
    } else {
        CellValue::Bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn table_cells_are_shown_as_the_server_sent_them() {
        let cell = |name: &str, text: &str| {
            table_cell(name.into(), text.as_bytes().to_vec(), ValueType::Text)
        };
        assert_eq!(
            cell("DATE", "0000-00-00"),
            CellValue::Date("0000-00-00".into())
        );
        assert_eq!(
            cell("DATETIME", "2026-10-20 08:05:09.120"),
            CellValue::DateTime("2026-10-20 08:05:09.120".into())
        );
        assert_eq!(
            cell("TIME", "-838:59:59"),
            CellValue::Time("-838:59:59".into())
        );
        assert_eq!(
            cell("BIGINT UNSIGNED", "18446744073709551615"),
            CellValue::UInt(u64::MAX)
        );
        assert_eq!(cell("FLOAT", "0.1"), CellValue::Float(0.1));
        assert_eq!(cell("SET", "r,w"), CellValue::Text("r,w".into()));
        assert_eq!(
            cell("GEOMETRY", "\0\0"),
            CellValue::unsupported("GEOMETRY"),
            "显示不了的类型不让改"
        );
        assert_eq!(
            table_cell("VARBINARY".into(), b"abc".to_vec(), ValueType::Binary),
            CellValue::Bytes(b"abc".to_vec()),
            "真正的二进制列不因内容碰巧是文本就显示成文本"
        );
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
