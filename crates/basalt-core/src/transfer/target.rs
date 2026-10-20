//! 导入时目标列的类型分类:按方言解析列的声明类型,决定客户端怎么校验和转换。
//!
//! 客户端校验是为了不让数据被悄悄改掉:MySQL 非严格模式会把超范围的数截成最大值、把太长的
//! 文本截断、把不合法的日期存成零日期;SQLite 什么都收。PostgreSQL 自己会报错,但客户端先查
//! 能给出更明白的原因。认不出的类型交给数据库解析([`TargetKind::Other`])。

use crate::sql::Dialect;
use crate::table::ColumnInfo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Integer {
        min: i128,
        max: i128,
    },
    /// MySQL 的 YEAR:0 或 1901–2155。
    Year,
    /// `finite_only`:MySQL、SQLite 存不了 NaN 和无穷大。
    Float {
        finite_only: bool,
    },
    /// `specials`:PostgreSQL 的 numeric 能存 NaN、Infinity。
    Decimal {
        precision: Option<u32>,
        scale: Option<u32>,
        specials: bool,
    },
    Bit {
        bits: u32,
    },
    Bool,
    Binary,
    Date,
    /// `long`:MySQL 的 TIME 可以是负数、可以超过 24 小时;`zoned`:PostgreSQL 的 timetz。
    Time {
        zoned: bool,
        long: bool,
    },
    DateTime {
        zoned: bool,
    },
    Json,
    Enum(Vec<String>),
    Set(Vec<String>),
    Text {
        max_chars: Option<u32>,
        max_bytes: Option<u64>,
    },
    /// 原样以文本交给数据库,由它按列类型解析。
    Other,
    /// SQLite 没有声明类型的列,什么都能存:规范写法的数字按数字存(导出时就是这么写的),
    /// 其余按文本存。
    Dynamic,
}

/// `int(10) unsigned` → (`int`, Some(`10`))。类型名转小写。
fn split_type(data_type: &str) -> (String, Option<&str>) {
    match data_type.split_once('(') {
        Some((base, rest)) => {
            let args = rest.rsplit_once(')').map_or(rest, |(args, _)| args);
            (base.trim().to_ascii_lowercase(), Some(args))
        }
        None => {
            let lower = data_type.to_ascii_lowercase();
            let base = lower
                .split(" unsigned")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            (base, None)
        }
    }
}

fn numbers(args: Option<&str>) -> Vec<u32> {
    args.unwrap_or_default()
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect()
}

/// `'a','it''s'` → [a, it's]。MySQL 的 ENUM / SET 成员列表。
fn quoted_list(args: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut chars = args.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\'' {
            continue;
        }
        let mut item = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\'' if chars.peek() == Some(&'\'') => {
                    chars.next();
                    item.push('\'');
                }
                '\'' => break,
                '\\' => item.extend(chars.next()),
                c => item.push(c),
            }
        }
        items.push(item);
    }
    items
}

fn signed(bits: u32) -> TargetKind {
    TargetKind::Integer {
        min: -(1_i128 << (bits - 1)),
        max: (1_i128 << (bits - 1)) - 1,
    }
}

fn unsigned(bits: u32) -> TargetKind {
    TargetKind::Integer {
        min: 0,
        max: (1_i128 << bits) - 1,
    }
}

fn integer(bits: u32, is_unsigned: bool) -> TargetKind {
    if is_unsigned {
        unsigned(bits)
    } else {
        signed(bits)
    }
}

fn mysql(data_type: &str) -> TargetKind {
    let (base, args) = split_type(data_type);
    let is_unsigned = data_type.to_ascii_lowercase().contains("unsigned");
    let numbers = numbers(args);
    match base.as_str() {
        "tinyint" => integer(8, is_unsigned),
        "smallint" => integer(16, is_unsigned),
        "mediumint" => integer(24, is_unsigned),
        "int" | "integer" => integer(32, is_unsigned),
        "bigint" => integer(64, is_unsigned),
        "year" => TargetKind::Year,
        "float" | "double" | "real" | "double precision" => TargetKind::Float { finite_only: true },
        "decimal" | "numeric" | "dec" | "fixed" => TargetKind::Decimal {
            precision: Some(numbers.first().copied().unwrap_or(10)),
            scale: Some(numbers.get(1).copied().unwrap_or(0)),
            specials: false,
        },
        "bit" => TargetKind::Bit {
            bits: numbers.first().copied().unwrap_or(1),
        },
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => {
            TargetKind::Binary
        }
        "date" => TargetKind::Date,
        "time" => TargetKind::Time {
            zoned: false,
            long: true,
        },
        "datetime" | "timestamp" => TargetKind::DateTime { zoned: false },
        "json" => TargetKind::Json,
        "enum" => TargetKind::Enum(quoted_list(args.unwrap_or_default())),
        "set" => TargetKind::Set(quoted_list(args.unwrap_or_default())),
        "char" | "varchar" => TargetKind::Text {
            max_chars: numbers.first().copied(),
            max_bytes: None,
        },
        "tinytext" | "text" | "mediumtext" => TargetKind::Text {
            max_chars: None,
            max_bytes: Some(match base.as_str() {
                "tinytext" => 255,
                "text" => 65_535,
                _ => 16_777_215,
            }),
        },
        "longtext" => TargetKind::Text {
            max_chars: None,
            max_bytes: None,
        },
        _ => TargetKind::Other,
    }
}

fn postgres(data_type: &str) -> TargetKind {
    let lower = data_type.to_ascii_lowercase();
    if lower.ends_with("[]") {
        return TargetKind::Other;
    }
    let (base, args) = split_type(&lower);
    let numbers = numbers(args);
    let zoned = lower.contains("with time zone");
    match base.as_str() {
        "smallint" => signed(16),
        "integer" => signed(32),
        "bigint" => signed(64),
        "real" | "double precision" => TargetKind::Float { finite_only: false },
        "numeric" => TargetKind::Decimal {
            precision: numbers.first().copied(),
            scale: numbers.get(1).copied().or(numbers.first().map(|_| 0)),
            specials: true,
        },
        "boolean" => TargetKind::Bool,
        "bytea" => TargetKind::Binary,
        "date" => TargetKind::Date,
        "time" | "time without time zone" | "time with time zone" => {
            TargetKind::Time { zoned, long: false }
        }
        "timestamp" | "timestamp without time zone" | "timestamp with time zone" => {
            TargetKind::DateTime { zoned }
        }
        "json" | "jsonb" => TargetKind::Json,
        "character varying" | "character" => TargetKind::Text {
            max_chars: numbers.first().copied(),
            max_bytes: None,
        },
        "text" => TargetKind::Text {
            max_chars: None,
            max_bytes: None,
        },
        _ => TargetKind::Other,
    }
}

/// SQLite 按声明类型里的关键字定亲和性(顺序有意义,和 SQLite 一致);NUMERIC 亲和性里再认出日期、布尔。
fn sqlite(declared: &str) -> TargetKind {
    let upper = declared.to_ascii_uppercase();
    let has = |keys: &[&str]| keys.iter().any(|k| upper.contains(k));
    if upper.trim().is_empty() {
        TargetKind::Dynamic
    } else if has(&["INT"]) {
        signed(64)
    } else if has(&["CHAR", "CLOB", "TEXT"]) {
        TargetKind::Text {
            max_chars: None,
            max_bytes: None,
        }
    } else if has(&["BLOB"]) {
        TargetKind::Binary
    } else if has(&["REAL", "FLOA", "DOUB"]) {
        TargetKind::Float { finite_only: true }
    } else if has(&["BOOL"]) {
        TargetKind::Bool
    } else if has(&["DATETIME", "TIMESTAMP"]) {
        TargetKind::DateTime { zoned: true }
    } else if has(&["DATE"]) {
        TargetKind::Date
    } else if has(&["TIME"]) {
        TargetKind::Time {
            zoned: false,
            long: false,
        }
    } else if has(&["DECIMAL", "NUMERIC"]) {
        TargetKind::Decimal {
            precision: None,
            scale: None,
            specials: false,
        }
    } else {
        TargetKind::Other
    }
}

pub fn classify(dialect: Dialect, column: &ColumnInfo) -> TargetKind {
    match dialect {
        Dialect::Mysql => mysql(&column.data_type),
        Dialect::Postgres => postgres(&column.data_type),
        Dialect::Sqlite => sqlite(&column.data_type),
    }
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
