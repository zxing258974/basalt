//! 导入时日期、时间、日期时间的校验与规范化(见 [`super::convert`])。
//!
//! MySQL 非严格模式会把不合法的日期悄悄存成零日期,PostgreSQL 不带时区的类型会悄悄丢掉
//! 时区偏移,所以客户端先查。各库特有的写法照认:MySQL 的零日期、PostgreSQL 的 infinity 与 BC。

use chrono::Timelike;

use super::source::SourceValue;
use super::temporal::{format_datetime, format_time, parse_clock, split_date};
use crate::sql::Dialect;

/// PostgreSQL 的 infinity、-infinity 与公元前的 BC 后缀。
fn postgres_special(dialect: Dialect, text: &str) -> bool {
    dialect == Dialect::Postgres
        && matches!(text.to_ascii_lowercase().as_str(), "infinity" | "-infinity")
}

pub(super) fn date(dialect: Dialect, value: &SourceValue) -> Result<String, String> {
    let fail = || "不是有效的日期（年-月-日）".to_owned();
    let text = match value {
        SourceValue::DateTime(value)
            if value.num_seconds_from_midnight() == 0 && value.nanosecond() == 0 =>
        {
            return Ok(value.date().format("%Y-%m-%d").to_string());
        }
        SourceValue::DateTime(_) => return Err("带有时间部分，这一列只存日期".into()),
        SourceValue::Text(text) => text.trim(),
        _ => return Err(fail()),
    };
    if postgres_special(dialect, text) {
        return Ok(text.to_owned());
    }
    let (body, era) = match text.strip_suffix(" BC") {
        Some(body) if dialect == Dialect::Postgres => (body, " BC"),
        _ => (text, ""),
    };
    let (parsed, rest) = split_date(body).ok_or_else(fail)?;
    let midnight = rest.is_empty()
        || parse_clock(rest.trim_start_matches([' ', 'T'])).is_some_and(|c| {
            c.hour == 0 && c.minute == 0 && c.second == 0 && c.fraction.trim_matches('0').is_empty()
        });
    if !midnight {
        return Err("带有时间部分，这一列只存日期".into());
    }
    match parsed {
        Ok(date) => Ok(format!("{}{era}", date.format("%Y-%m-%d"))),
        Err(()) if dialect == Dialect::Mysql && body.starts_with("0000-00-00") => {
            Ok("0000-00-00".into())
        }
        Err(()) => Err(fail()),
    }
}

/// 时间后面的时区偏移:`Z`、`+08`、`+08:00`、`-0530`。返回 (去掉偏移的部分, 偏移)。
fn split_offset(text: &str) -> (&str, &str) {
    if let Some(body) = text.strip_suffix('Z') {
        return (body, "Z");
    }
    match text.rfind(['+', '-']) {
        Some(at) if at > 0 => {
            let offset = &text[at + 1..];
            let valid = matches!(offset.len(), 2 | 4 | 5)
                && offset
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_digit() || (i == 2 && b == b':'));
            if valid {
                (&text[..at], &text[at..])
            } else {
                (text, "")
            }
        }
        _ => (text, ""),
    }
}

/// 秒的小数位超过列的精度时数据库会四舍五入(23:59:59.6 存进 DATETIME 变成第二天),不能悄悄进位。
fn check_fraction(fraction: &str, fsp: Option<u32>) -> Result<(), String> {
    let digits = fraction.trim_end_matches('0').len();
    match fsp {
        Some(fsp) if digits > fsp as usize => Err(if fsp == 0 {
            "带有秒的小数，这一列只精确到秒（数据库会四舍五入）".into()
        } else {
            format!("秒的小数超过这一列的 {fsp} 位（数据库会四舍五入）")
        }),
        _ => Ok(()),
    }
}

/// `fsp` 是列能存的秒的小数位数,`None` 是不限。
pub(super) fn time(
    value: &SourceValue,
    zoned: bool,
    long: bool,
    fsp: Option<u32>,
) -> Result<String, String> {
    let fail = || "不是有效的时间（时:分:秒）".to_owned();
    let owned;
    let text = match value {
        SourceValue::DateTime(value) => {
            let date = value.date().to_string();
            if date != "1899-12-30" && date != "1899-12-31" {
                return Err("是日期时间，不是时间".into());
            }
            owned = format_time(value.time());
            owned.as_str()
        }
        SourceValue::Text(text) => text.trim(),
        _ => return Err(fail()),
    };
    let body = if zoned { split_offset(text).0 } else { text };
    let clock = parse_clock(body).ok_or_else(fail)?;
    let max_hour = if long { 838 } else { 24 };
    if clock.hour > max_hour || (clock.negative && !long) {
        return Err(fail());
    }
    check_fraction(clock.fraction, fsp)?;
    Ok(text.to_owned())
}

pub(super) fn datetime(
    dialect: Dialect,
    value: &SourceValue,
    zoned: bool,
    fsp: Option<u32>,
) -> Result<String, String> {
    let fail = || "不是有效的日期时间（年-月-日 时:分:秒）".to_owned();
    let owned;
    let text = match value {
        SourceValue::DateTime(value) => {
            owned = format_datetime(*value);
            owned.as_str()
        }
        SourceValue::Text(text) => text.trim(),
        _ => return Err(fail()),
    };
    if postgres_special(dialect, text) {
        return Ok(text.to_owned());
    }
    let (body, era) = match text.strip_suffix(" BC") {
        Some(body) if dialect == Dialect::Postgres => (body, " BC"),
        _ => (text, ""),
    };
    let (parsed, rest) = split_date(body).ok_or_else(fail)?;
    let zero_date = dialect == Dialect::Mysql && body.starts_with("0000-00-00");
    let date = match parsed {
        Ok(date) => date.format("%Y-%m-%d").to_string(),
        Err(()) if zero_date => "0000-00-00".into(),
        Err(()) => return Err(fail()),
    };
    if rest.is_empty() {
        return Ok(format!("{date} 00:00:00{era}"));
    }
    let clock_text = rest.strip_prefix([' ', 'T']).ok_or_else(fail)?;
    let (clock_text, offset) = split_offset(clock_text);
    if !offset.is_empty() && !zoned {
        return Err("带有时区偏移，这一列不带时区，数据库会把偏移丢掉".into());
    }
    let clock = parse_clock(clock_text).ok_or_else(fail)?;
    if clock.negative || clock.hour > 23 {
        return Err(fail());
    }
    check_fraction(clock.fraction, fsp)?;
    Ok(format!("{date} {clock_text}{offset}{era}"))
}

#[cfg(test)]
#[path = "convert_temporal_tests.rs"]
mod tests;
