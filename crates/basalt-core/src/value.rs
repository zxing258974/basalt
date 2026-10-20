//! 统一的单元格值。驱动把每个单元格解码成 [`CellValue`],前端只按它渲染与编辑。
//!
//! 序列化成 `{ "kind": "...", "value": ... }`。整数、浮点数一律写成字符串:
//! JavaScript 的 number 只能精确表示 2^53 以内的整数,BIGINT 主键会被悄悄改掉;
//! NaN / Infinity 也没法放进 JSON。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CellValue {
    Null,
    Bool(bool),
    Int(#[serde(with = "as_string")] i64),
    #[serde(rename = "uint")]
    UInt(#[serde(with = "as_string")] u64),
    Float(#[serde(with = "float_string")] f64),
    /// 定点数原样保留数据库给出的文本,不经过二进制浮点。
    Decimal(String),
    Text(String),
    /// 序列化为小写十六进制,不带 0x 前缀。
    Bytes(#[serde(with = "hex_string")] Vec<u8>),
    /// JSON 文本,保留数据库给出的写法。
    Json(String),
    Date(String),
    Time(String),
    DateTime(String),
    /// 解不了的类型:显示类型名,不能编辑。
    #[serde(rename_all = "camelCase")]
    Unsupported {
        type_name: String,
    },
}

impl CellValue {
    pub fn unsupported(type_name: &str) -> Self {
        Self::Unsupported {
            type_name: type_name.to_owned(),
        }
    }
}

/// 浮点数的文本形式:用 Rust 的最短可回读表示,特殊值用 PostgreSQL 的写法,
/// 这样写回时 `'Infinity'::float8` 之类能被数据库原样解析。
pub fn format_float(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else {
        value.to_string()
    }
}

/// 单精度值先按 f32 的最短表示转文本再转 f64,避免显示成 0.10000000149011612。
pub fn widen_f32(value: f32) -> f64 {
    if value.is_finite() {
        value.to_string().parse().unwrap_or(f64::from(value))
    } else {
        f64::from(value)
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// 解析十六进制文本,允许大小写混用;长度必须是偶数。
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect()
}

mod as_string {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<T: Display, S: Serializer>(
        value: &T,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(value)
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        text.trim().parse().map_err(D::Error::custom)
    }
}

mod float_string {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::format_float(*value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.trim().parse().map_err(D::Error::custom)
    }
}

mod hex_string {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::to_hex(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        super::from_hex(&text).ok_or_else(|| D::Error::custom("不是有效的十六进制"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn round_trip(value: &CellValue) -> CellValue {
        serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
    }

    #[test]
    fn big_integers_travel_as_exact_strings() {
        let id = CellValue::Int(9_007_199_254_740_993);
        assert_eq!(
            serde_json::to_value(&id).unwrap(),
            json!({ "kind": "int", "value": "9007199254740993" })
        );
        assert_eq!(round_trip(&id), id);
        let max = CellValue::UInt(u64::MAX);
        assert_eq!(
            serde_json::to_value(&max).unwrap(),
            json!({ "kind": "uint", "value": "18446744073709551615" })
        );
        assert_eq!(round_trip(&max), max);
    }

    #[test]
    fn null_and_unsupported_have_stable_shapes() {
        assert_eq!(
            serde_json::to_value(CellValue::Null).unwrap(),
            json!({ "kind": "null" })
        );
        let parsed: CellValue = serde_json::from_value(json!({ "kind": "null" })).unwrap();
        assert_eq!(parsed, CellValue::Null);
        assert_eq!(
            serde_json::to_value(CellValue::unsupported("GEOMETRY")).unwrap(),
            json!({ "kind": "unsupported", "value": { "typeName": "GEOMETRY" } })
        );
        assert_eq!(
            serde_json::to_value(CellValue::DateTime("2026-10-20 08:00:00".into())).unwrap(),
            json!({ "kind": "dateTime", "value": "2026-10-20 08:00:00" })
        );
    }

    #[test]
    fn floats_keep_special_values_and_short_form() {
        assert_eq!(
            serde_json::to_value(CellValue::Float(f64::NAN)).unwrap(),
            json!({ "kind": "float", "value": "NaN" })
        );
        assert_eq!(format_float(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(format_float(0.1), "0.1");
        assert_eq!(widen_f32(0.1_f32), 0.1_f64);
        let inf = round_trip(&CellValue::Float(f64::INFINITY));
        assert_eq!(inf, CellValue::Float(f64::INFINITY));
    }

    #[test]
    fn bytes_are_lowercase_hex_and_reject_bad_input() {
        let bytes = CellValue::Bytes(vec![0x00, 0xab, 0x7f]);
        assert_eq!(
            serde_json::to_value(&bytes).unwrap(),
            json!({ "kind": "bytes", "value": "00ab7f" })
        );
        assert_eq!(round_trip(&bytes), bytes);
        assert_eq!(from_hex("0A1b"), Some(vec![0x0a, 0x1b]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex(""), Some(vec![]));
    }
}
