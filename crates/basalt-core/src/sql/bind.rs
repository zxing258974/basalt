//! 参数值:用户输入与主键原值怎么变成绑定参数,以及预览里怎么写成字面量。

use rust_decimal::Decimal;

use super::Dialect;
use crate::table::ValueType;
use crate::value::{CellValue, format_float, from_hex, to_hex};

#[derive(Clone, Debug, PartialEq)]
pub enum BindValue {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Decimal(Decimal),
    Text(String),
    Bytes(Vec<u8>),
}

/// 显示用的字面量。MySQL 默认把反斜杠当转义符,所以要多转一层。
pub(crate) fn literal(dialect: Dialect, value: &BindValue) -> String {
    match value {
        BindValue::Null => "NULL".into(),
        BindValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        BindValue::Int(n) => n.to_string(),
        BindValue::UInt(n) => n.to_string(),
        BindValue::Float(f) if f.is_finite() => format_float(*f),
        BindValue::Float(f) => quote_string(dialect, &format_float(*f)),
        BindValue::Decimal(d) => d.to_string(),
        BindValue::Text(text) => quote_string(dialect, text),
        BindValue::Bytes(bytes) => match dialect {
            Dialect::Postgres => format!("'\\x{}'", to_hex(bytes)),
            Dialect::Mysql | Dialect::Sqlite => format!("X'{}'", to_hex(bytes)),
        },
    }
}

pub(crate) fn quote_string(dialect: Dialect, text: &str) -> String {
    let escaped = match dialect {
        Dialect::Mysql => text.replace('\\', "\\\\").replace('\'', "''"),
        Dialect::Postgres | Dialect::Sqlite => text.replace('\'', "''"),
    };
    format!("'{escaped}'")
}

/// 编辑框里的文本 → 参数。PostgreSQL 一律传文本让数据库按列类型解析;
/// SQLite 按列亲和性自己转换,也传文本;MySQL 在客户端先校验数值,
/// 免得非严格模式下把错误输入悄悄存成 0。
pub(super) fn convert_input(
    dialect: Dialect,
    value_type: ValueType,
    text: &str,
) -> Result<BindValue, &'static str> {
    if value_type == ValueType::Binary
        && let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))
    {
        let bytes = from_hex(hex).ok_or("不是有效的十六进制")?;
        return Ok(match dialect {
            Dialect::Postgres => BindValue::Text(format!("\\x{}", to_hex(&bytes))),
            Dialect::Mysql | Dialect::Sqlite => BindValue::Bytes(bytes),
        });
    }
    if dialect != Dialect::Mysql {
        return Ok(BindValue::Text(text.to_owned()));
    }
    let trimmed = text.trim();
    Ok(match value_type {
        ValueType::Integer { unsigned: false } => {
            BindValue::Int(trimmed.parse().map_err(|_| "不是有效的整数")?)
        }
        ValueType::Integer { unsigned: true } => {
            BindValue::UInt(trimmed.parse().map_err(|_| "不是有效的无符号整数")?)
        }
        ValueType::Bit => BindValue::UInt(
            trimmed
                .parse()
                .map_err(|_| "不是有效的位值（请填非负整数）")?,
        ),
        ValueType::Float => match trimmed.parse::<f64>() {
            Ok(number) if number.is_finite() => BindValue::Float(number),
            _ => return Err("不是有效的数字（MySQL 不支持 NaN 和无穷大）"),
        },
        ValueType::Decimal if is_decimal_literal(trimmed) => BindValue::Text(trimmed.to_owned()),
        ValueType::Decimal => return Err("不是有效的定点数"),
        ValueType::Binary | ValueType::Text => BindValue::Text(text.to_owned()),
    })
}

/// `[+-]` 数字 `[.数字]` `[e[+-]数字]`,小数点两侧至少有一边是数字。
fn is_decimal_literal(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mantissa_ok =
        digits(whole) && digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && digits(e)
    });
    mantissa_ok && exponent_ok
}

/// 主键原值 → 定位参数。返回 `None` 表示这种值不能用来定位(解不了的类型)。
pub(super) fn key_param(dialect: Dialect, value: &CellValue) -> Option<BindValue> {
    if dialect == Dialect::Postgres {
        return key_text(value).map(BindValue::Text);
    }
    Some(match value {
        CellValue::Null | CellValue::Unsupported { .. } => return None,
        CellValue::Bool(b) => BindValue::Int(i64::from(*b)),
        CellValue::Int(n) => BindValue::Int(*n),
        CellValue::UInt(n) => match (dialect, i64::try_from(*n)) {
            (Dialect::Sqlite, Ok(n)) => BindValue::Int(n),
            (Dialect::Sqlite, Err(_)) => BindValue::Text(n.to_string()),
            _ => BindValue::UInt(*n),
        },
        CellValue::Float(f) => BindValue::Float(*f),
        // MySQL 拿字符串和 DECIMAL 比较时按浮点比,必须以定点数传入。
        CellValue::Decimal(text) => match dialect {
            Dialect::Mysql => BindValue::Decimal(Decimal::from_str_exact(text).ok()?),
            _ => BindValue::Text(text.clone()),
        },
        CellValue::Bytes(bytes) => BindValue::Bytes(bytes.clone()),
        CellValue::Text(text)
        | CellValue::Json(text)
        | CellValue::Date(text)
        | CellValue::Time(text)
        | CellValue::DateTime(text) => BindValue::Text(text.clone()),
    })
}

/// PostgreSQL 的文本输入形式,配合 `::类型` 由数据库解析。
fn key_text(value: &CellValue) -> Option<String> {
    Some(match value {
        CellValue::Null | CellValue::Unsupported { .. } => return None,
        CellValue::Bool(b) => b.to_string(),
        CellValue::Int(n) => n.to_string(),
        CellValue::UInt(n) => n.to_string(),
        CellValue::Float(f) => format_float(*f),
        CellValue::Bytes(bytes) => format!("\\x{}", to_hex(bytes)),
        CellValue::Decimal(text)
        | CellValue::Text(text)
        | CellValue::Json(text)
        | CellValue::Date(text)
        | CellValue::Time(text)
        | CellValue::DateTime(text) => text.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::dml::build_changes;
    use crate::sql::test_support::*;
    use crate::table::{ColumnInfo, RowChange, TableRef};

    #[test]
    fn decimal_literals_follow_sql_number_syntax() {
        for ok in ["1", "-1.5", "+.5", "3.", "1e10", "2.5E-3", "007"] {
            assert!(is_decimal_literal(ok), "{ok}");
        }
        for bad in ["", ".", "1.2.3", "1e", "e5", "12,5", "0x10", "- 1", "1e+"] {
            assert!(!is_decimal_literal(bad), "{bad}");
        }
    }

    #[test]
    fn preview_escapes_quotes_and_mysql_backslashes() {
        let (table, columns) = mysql_users();
        let change = RowChange::Update {
            key: key(&[("id", CellValue::Int(1))]),
            values: values(&[("name", Some("O'Brien \\ 张"))]),
        };
        assert_eq!(
            one(Dialect::Mysql, &table, &columns, change).preview(),
            "UPDATE `shop`.`users` SET `name` = 'O''Brien \\\\ 张' WHERE `id` = 1"
        );
        let (pg_table, pg_columns) = pg_users();
        let change = RowChange::Update {
            key: key(&[("id", CellValue::Int(1))]),
            values: values(&[("name", Some("O'Brien \\ 张"))]),
        };
        assert_eq!(
            one(Dialect::Postgres, &pg_table, &pg_columns, change).preview(),
            "UPDATE \"public\".\"users\" SET \"name\" = 'O''Brien \\ 张' WHERE \"id\" = '1'::integer"
        );
    }

    #[test]
    fn binary_columns_take_hex_with_0x_prefix() {
        let (table, columns) = mysql_users();
        let statement = one(
            Dialect::Mysql,
            &table,
            &columns,
            RowChange::Update {
                key: key(&[("id", CellValue::Int(1))]),
                values: values(&[("avatar", Some("0x0A1b"))]),
            },
        );
        assert_eq!(statement.params()[0], BindValue::Bytes(vec![0x0a, 0x1b]));
        assert!(
            statement.preview().contains("`avatar` = X'0a1b'"),
            "{}",
            statement.preview()
        );

        let statement = one(
            Dialect::Mysql,
            &table,
            &columns,
            RowChange::Update {
                key: key(&[("id", CellValue::Int(1))]),
                values: values(&[("avatar", Some("plain"))]),
            },
        );
        assert_eq!(statement.params()[0], BindValue::Text("plain".into()));

        let (pg_table, pg_columns) = pg_users();
        let statement = one(
            Dialect::Postgres,
            &pg_table,
            &pg_columns,
            RowChange::Update {
                key: key(&[("id", CellValue::Int(1))]),
                values: values(&[("photo", Some("0xFF00"))]),
            },
        );
        assert_eq!(statement.params()[0], BindValue::Text("\\xff00".into()));
        assert!(
            statement.preview().contains("\"photo\" = '\\xff00'::bytea"),
            "{}",
            statement.preview()
        );
    }

    #[test]
    fn postgres_and_sqlite_keys_keep_exact_text_forms() {
        let (pg_table, pg_columns) = pg_users();
        let statement = one(
            Dialect::Postgres,
            &pg_table,
            &pg_columns,
            RowChange::Delete {
                key: key(&[("id", CellValue::Int(9_007_199_254_740_993))]),
            },
        );
        assert_eq!(
            statement.params(),
            [BindValue::Text("9007199254740993".into())]
        );

        let table = TableRef {
            database: "main".into(),
            schema: None,
            table: "blobs".into(),
        };
        let columns = [ColumnInfo {
            is_primary_key: true,
            ..ColumnInfo::new("digest", "BLOB")
        }];
        let statement = one(
            Dialect::Sqlite,
            &table,
            &columns,
            RowChange::Delete {
                key: key(&[("digest", CellValue::Bytes(vec![1, 2]))]),
            },
        );
        assert_eq!(statement.params(), [BindValue::Bytes(vec![1, 2])]);
        assert_eq!(
            statement.preview(),
            "DELETE FROM \"main\".\"blobs\" WHERE \"digest\" = X'0102'"
        );
    }

    #[test]
    fn reports_conversion_errors_with_change_number_and_column() {
        let (table, columns) = mysql_users();
        let changes = [
            RowChange::Insert {
                values: values(&[("name", Some("ok"))]),
            },
            RowChange::Update {
                key: key(&[("id", CellValue::Int(1))]),
                values: values(&[("age", Some("4x2"))]),
            },
        ];
        let error = build_changes(Dialect::Mysql, &table, &columns, &changes).unwrap_err();
        assert_eq!(error.code(), "invalid_edit");
        let message = error.to_string();
        assert!(
            message.contains("「age」") && message.contains("整数"),
            "{message}"
        );

        for (column, input) in [
            ("visits", "-1"),
            ("balance", "12,5"),
            ("score", "NaN"),
            ("flags", "x"),
        ] {
            let change = RowChange::Update {
                key: key(&[("id", CellValue::Int(1))]),
                values: values(&[(column, Some(input))]),
            };
            let error = build_changes(Dialect::Mysql, &table, &columns, &[change]).unwrap_err();
            assert_eq!(error.code(), "invalid_edit", "{column} = {input}");
        }
    }
}
