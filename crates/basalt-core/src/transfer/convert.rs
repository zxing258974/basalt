//! 导入时把来源的值按目标列转成绑定参数。解析不了就返回原因(进失败报告),
//! 绝不改成 NULL 或别的值。转换结果按方言交给驱动:PostgreSQL 一律传文本、`::类型` 由数据库解析
//! (和表格编辑一样);MySQL、SQLite 按类型绑定。

use serde::de::IgnoredAny;

use super::BinaryEncoding;
use super::convert_temporal::{date, datetime, time};
use super::render::parse_binary_text;
use super::source::{SourceValue, describe_datetime};
use super::target::TargetKind;
use crate::sql::Dialect;
use crate::sql::bind::BindValue;
use crate::table::ColumnInfo;
use crate::value::{format_float, to_hex};

/// 2^53:比它大的整数,双精度浮点(Excel 的数字)存不精确。
const EXACT_FLOAT_INTEGER: f64 = 9_007_199_254_740_992.0;

/// 转换好、还没按方言定绑定方式的值。
#[derive(Clone, Debug, PartialEq)]
enum Typed {
    Int(i128),
    /// 浮点数与它的文本(PostgreSQL 用文本)。
    Float(f64, String),
    Decimal(String),
    Bool(bool),
    Bytes(Vec<u8>),
    Text(String),
}

fn bind(dialect: Dialect, typed: Typed) -> BindValue {
    match (dialect, typed) {
        (Dialect::Postgres, Typed::Int(n)) => BindValue::Text(n.to_string()),
        (Dialect::Postgres, Typed::Float(_, text)) => BindValue::Text(text),
        (Dialect::Postgres, Typed::Bool(b)) => BindValue::Text(b.to_string()),
        (Dialect::Postgres, Typed::Bytes(bytes)) => {
            BindValue::Text(format!("\\x{}", to_hex(&bytes)))
        }
        (_, Typed::Int(n)) => match i64::try_from(n) {
            Ok(n) => BindValue::Int(n),
            Err(_) => {
                u64::try_from(n).map_or_else(|_| BindValue::Text(n.to_string()), BindValue::UInt)
            }
        },
        (_, Typed::Float(f, _)) => BindValue::Float(f),
        (_, Typed::Bool(b)) => BindValue::Int(i64::from(b)),
        (_, Typed::Bytes(bytes)) => BindValue::Bytes(bytes),
        (_, Typed::Decimal(text) | Typed::Text(text)) => BindValue::Text(text),
    }
}

/// 一个来源值写进目标列。返回的 `Err` 是给人看的原因,不含值本身(报告里另有一栏)。
pub fn convert(
    dialect: Dialect,
    column: &ColumnInfo,
    kind: &TargetKind,
    value: &SourceValue,
    binary: BinaryEncoding,
) -> Result<BindValue, String> {
    match value {
        SourceValue::Null if column.nullable => return Ok(BindValue::Null),
        SourceValue::Null => return Err("这一列不能为空".into()),
        SourceValue::Error(error) => return Err(format!("是 Excel 的错误值 {error}")),
        _ => {}
    }
    let typed = match kind {
        TargetKind::Integer { min, max } => Typed::Int(integer(value, *min, *max)?),
        TargetKind::Year => Typed::Int(year(value)?),
        TargetKind::Float { finite_only } => float(value, *finite_only)?,
        TargetKind::Decimal {
            precision,
            scale,
            specials,
        } => Typed::Decimal(decimal(value, *precision, *scale, *specials)?),
        TargetKind::Bit { bits } => Typed::Int(bit(value, *bits)?),
        TargetKind::Bool => Typed::Bool(boolean(value)?),
        TargetKind::Binary => Typed::Bytes(bytes(value, binary)?),
        TargetKind::Date => Typed::Text(date(dialect, value)?),
        TargetKind::Time { zoned, long } => Typed::Text(time(value, *zoned, *long)?),
        TargetKind::DateTime { zoned } => Typed::Text(datetime(dialect, value, *zoned)?),
        TargetKind::Json => Typed::Text(json(value)?),
        TargetKind::Enum(members) => Typed::Text(member(&text_of(value), members)?),
        TargetKind::Set(members) => Typed::Text(set(&text_of(value), members)?),
        TargetKind::Text {
            max_chars,
            max_bytes,
        } => Typed::Text(limited(text_of(value), *max_chars, *max_bytes)?),
        TargetKind::Other => Typed::Text(text_of(value)),
    };
    Ok(bind(dialect, typed))
}

/// 值的文本形式。Excel 的数字用最短写法(整数不带 .0),日期按 [`describe_datetime`]。
fn text_of(value: &SourceValue) -> String {
    match value {
        SourceValue::Text(text) => text.clone(),
        SourceValue::DateTime(value) => describe_datetime(*value),
        other => other.display().unwrap_or_default(),
    }
}

fn integral(number: f64) -> Result<i128, String> {
    if !number.is_finite() || number.fract() != 0.0 {
        return Err("不是整数".into());
    }
    if number.abs() > EXACT_FLOAT_INTEGER {
        return Err(
            "超过 15 位，Excel 里存的已经不是精确值；请把这一列设成文本格式后再导入".into(),
        );
    }
    // 上面已经确认是 2^53 以内的整数,转换是精确的。
    #[allow(clippy::cast_possible_truncation)]
    Ok(number as i128)
}

fn integer(value: &SourceValue, min: i128, max: i128) -> Result<i128, String> {
    let number = match value {
        SourceValue::Text(text) => text
            .trim()
            .parse::<i128>()
            .map_err(|_| "不是有效的整数".to_owned())?,
        SourceValue::Integer(n) => i128::from(*n),
        SourceValue::Number(f) => integral(*f)?,
        SourceValue::Bool(b) => i128::from(*b),
        _ => return Err("是日期，不是整数".into()),
    };
    if number < min || number > max {
        return Err(format!("超出这一列的范围（{min} 到 {max}）"));
    }
    Ok(number)
}

fn year(value: &SourceValue) -> Result<i128, String> {
    let number = integer(value, 0, 2155)?;
    if number != 0 && number < 1901 {
        return Err("年份要在 1901 到 2155 之间（或 0）".into());
    }
    Ok(number)
}

fn float(value: &SourceValue, finite_only: bool) -> Result<Typed, String> {
    let (number, text) = match value {
        SourceValue::Text(text) => {
            let trimmed = text.trim();
            let number: f64 = trimmed.parse().map_err(|_| "不是有效的数字".to_owned())?;
            let named = trimmed
                .trim_start_matches(['+', '-'])
                .starts_with(['i', 'I', 'n', 'N']);
            if !number.is_finite() && !named {
                return Err("超出浮点数的范围".into());
            }
            (number, trimmed.to_owned())
        }
        SourceValue::Number(f) => (*f, format_float(*f)),
        SourceValue::Integer(n) => {
            let f = *n as f64;
            if f.abs() > EXACT_FLOAT_INTEGER {
                return Err("整数太大，浮点数存不精确".into());
            }
            (f, n.to_string())
        }
        _ => return Err("不是数字".into()),
    };
    if finite_only && !number.is_finite() {
        return Err("这种数据库存不了 NaN 和无穷大".into());
    }
    Ok(Typed::Float(number, text))
}

/// 十进制文本的整数位数与小数位数(去掉前导零和末尾的零,按指数换算)。不是十进制数时返回 `None`。
pub fn decimal_digits(text: &str) -> Option<(i64, i64)> {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
        None => (unsigned, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !all_digits(whole) || !all_digits(fraction) {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let point = i64::try_from(whole.len()).ok()? + exponent;
    let first = digits.find(|c| c != '0');
    let last = digits.rfind(|c| c != '0');
    let (Some(first), Some(last)) = (first, last) else {
        return Some((0, 0));
    };
    let (first, last) = (i64::try_from(first).ok()?, i64::try_from(last).ok()?);
    Some(((point - first).max(0), (last + 1 - point).max(0)))
}

fn decimal(
    value: &SourceValue,
    precision: Option<u32>,
    scale: Option<u32>,
    specials: bool,
) -> Result<String, String> {
    let text = match value {
        SourceValue::Text(text) => text.trim().to_owned(),
        SourceValue::Number(f) if f.is_finite() => format_float(*f),
        SourceValue::Integer(n) => n.to_string(),
        _ => return Err("不是有效的数字".into()),
    };
    let named = text.trim_start_matches(['+', '-']).to_ascii_lowercase();
    if specials && ["nan", "infinity", "inf"].contains(&named.as_str()) {
        return Ok(text);
    }
    let (whole, fraction) = decimal_digits(&text).ok_or("不是有效的数字")?;
    if let (Some(precision), Some(scale)) = (precision, scale) {
        let room = i64::from(precision) - i64::from(scale);
        if whole > room {
            return Err(format!("超出这一列的范围（整数部分最多 {room} 位）"));
        }
        if fraction > i64::from(scale) {
            return Err(format!("小数位超过这一列的 {scale} 位"));
        }
    }
    Ok(text)
}

fn bit(value: &SourceValue, bits: u32) -> Result<i128, String> {
    let number = match value {
        SourceValue::Text(text) => {
            let trimmed = text.trim();
            match trimmed
                .strip_prefix("b'")
                .and_then(|t| t.strip_suffix('\''))
            {
                Some(binary) => {
                    i128::from_str_radix(binary, 2).map_err(|_| "不是有效的位值".to_owned())?
                }
                None => trimmed
                    .parse::<i128>()
                    .map_err(|_| "不是有效的位值（请填非负整数）".to_owned())?,
            }
        }
        other => integer(other, 0, i128::MAX)?,
    };
    let max = (1_i128 << bits.min(64)) - 1;
    if !(0..=max).contains(&number) {
        return Err(format!("超出 BIT({bits}) 的范围（0 到 {max}）"));
    }
    Ok(number)
}

fn boolean(value: &SourceValue) -> Result<bool, String> {
    let fail = || "不是布尔值（true / false、1 / 0）".to_owned();
    match value {
        SourceValue::Bool(b) => Ok(*b),
        SourceValue::Integer(0) => Ok(false),
        SourceValue::Integer(1) => Ok(true),
        SourceValue::Number(f) if *f == 0.0 => Ok(false),
        SourceValue::Number(f) if *f == 1.0 => Ok(true),
        SourceValue::Text(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "on" | "1" => Ok(true),
            "false" | "f" | "no" | "n" | "off" | "0" => Ok(false),
            _ => Err(fail()),
        },
        _ => Err(fail()),
    }
}

fn bytes(value: &SourceValue, binary: BinaryEncoding) -> Result<Vec<u8>, String> {
    let text = text_of(value);
    match binary {
        // 和表格里输入二进制一样:0x 开头的按十六进制,其余按文本的 UTF-8 字节。
        BinaryEncoding::Hex if !text.starts_with("0x") && !text.starts_with("0X") => {
            Ok(text.into_bytes())
        }
        _ => parse_binary_text(&text, binary).map_err(str::to_owned),
    }
}

fn json(value: &SourceValue) -> Result<String, String> {
    let text = match value {
        SourceValue::Bool(b) => b.to_string(),
        SourceValue::Number(f) if f.is_finite() => format_float(*f),
        other => text_of(other),
    };
    serde_json::from_str::<IgnoredAny>(&text).map_err(|_| "不是有效的 JSON".to_owned())?;
    Ok(text)
}

fn member(text: &str, members: &[String]) -> Result<String, String> {
    if members
        .iter()
        .any(|m| m == text || m.eq_ignore_ascii_case(text))
    {
        Ok(text.to_owned())
    } else {
        Err(format!("不是可选值（{}）之一", members.join("、")))
    }
}

fn set(text: &str, members: &[String]) -> Result<String, String> {
    if !text.is_empty() {
        for item in text.split(',') {
            member(item, members)?;
        }
    }
    Ok(text.to_owned())
}

fn limited(text: String, max_chars: Option<u32>, max_bytes: Option<u64>) -> Result<String, String> {
    if let Some(max) = max_chars
        && text.chars().count() > max as usize
    {
        return Err(format!("超过这一列的长度（最多 {max} 个字符）"));
    }
    if let Some(max) = max_bytes
        && text.len() as u64 > max
    {
        return Err(format!("超过这一列的长度（最多 {max} 字节）"));
    }
    Ok(text)
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
