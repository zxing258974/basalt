//! 单元格值写成文本的公共规则:CSV、Excel 里的文本形式,二进制的两种编码。

use std::borrow::Cow;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::BinaryEncoding;
use crate::value::{CellValue, format_float, from_hex, to_hex};

/// 二进制写成文本:`0x` 加小写十六进制,或标准 Base64。
pub fn binary_text(bytes: &[u8], encoding: BinaryEncoding) -> String {
    match encoding {
        BinaryEncoding::Hex => format!("0x{}", to_hex(bytes)),
        BinaryEncoding::Base64 => STANDARD.encode(bytes),
    }
}

/// [`binary_text`] 的逆运算,导入时用。十六进制要带 `0x`(大小写都行)。
pub fn parse_binary_text(text: &str, encoding: BinaryEncoding) -> Result<Vec<u8>, &'static str> {
    match encoding {
        BinaryEncoding::Hex => text
            .strip_prefix("0x")
            .or_else(|| text.strip_prefix("0X"))
            .and_then(from_hex)
            .ok_or("不是 0x 开头的有效十六进制"),
        BinaryEncoding::Base64 => STANDARD
            .decode(text.trim())
            .map_err(|_| "不是有效的 Base64"),
    }
}

/// 浮点数的文本:绝对值很大或很小时用科学计数法(1e300),免得写出几百位数字;都能原样读回。
pub fn float_text(value: f64) -> String {
    let magnitude = value.abs();
    if value.is_finite() && magnitude != 0.0 && !(1e-6..1e16).contains(&magnitude) {
        format!("{value:e}")
    } else {
        format_float(value)
    }
}

/// 驱动解不了的值没法如实导出,说清楚是什么类型。
pub fn unsupported_reason(type_name: &str) -> String {
    format!("是 {type_name} 类型，Basalt 读不出它的值，没法导出")
}

/// 值的纯文本形式,CSV 和查询结果写进 Excel 的文本都用它。NULL 为 `None`。
/// 数字保留全部位数;浮点数用最短的可回读写法,特殊值写成 NaN / Infinity。
pub fn plain_text(
    value: &CellValue,
    binary: BinaryEncoding,
) -> Result<Option<Cow<'_, str>>, String> {
    Ok(Some(match value {
        CellValue::Null => return Ok(None),
        CellValue::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
        CellValue::Int(n) => Cow::Owned(n.to_string()),
        CellValue::UInt(n) => Cow::Owned(n.to_string()),
        CellValue::Float(f) => Cow::Owned(float_text(*f)),
        CellValue::Bytes(bytes) => Cow::Owned(binary_text(bytes, binary)),
        CellValue::Decimal(text)
        | CellValue::Text(text)
        | CellValue::Json(text)
        | CellValue::Date(text)
        | CellValue::Time(text)
        | CellValue::DateTime(text) => Cow::Borrowed(text.as_str()),
        CellValue::Unsupported { type_name } => return Err(unsupported_reason(type_name)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_round_trips_through_both_encodings() {
        let bytes = [0x00, 0xff, 0x10, 0xab];
        assert_eq!(binary_text(&bytes, BinaryEncoding::Hex), "0x00ff10ab");
        assert_eq!(binary_text(&bytes, BinaryEncoding::Base64), "AP8Qqw==");
        for encoding in [BinaryEncoding::Hex, BinaryEncoding::Base64] {
            let text = binary_text(&bytes, encoding);
            assert_eq!(parse_binary_text(&text, encoding).unwrap(), bytes);
        }
        assert_eq!(binary_text(&[], BinaryEncoding::Hex), "0x");
        assert_eq!(
            parse_binary_text("0XAB", BinaryEncoding::Hex).unwrap(),
            [0xab]
        );
        assert!(parse_binary_text("ab", BinaryEncoding::Hex).is_err());
        assert!(parse_binary_text("0xabc", BinaryEncoding::Hex).is_err());
        assert!(parse_binary_text("不是", BinaryEncoding::Base64).is_err());
    }

    #[test]
    fn plain_text_keeps_every_digit_and_marks_null() {
        let hex = BinaryEncoding::Hex;
        assert_eq!(plain_text(&CellValue::Null, hex).unwrap(), None);
        assert_eq!(
            plain_text(&CellValue::UInt(u64::MAX), hex)
                .unwrap()
                .unwrap(),
            "18446744073709551615"
        );
        assert_eq!(
            plain_text(&CellValue::Decimal("12345678901234567890.1230".into()), hex)
                .unwrap()
                .unwrap(),
            "12345678901234567890.1230"
        );
        assert_eq!(
            plain_text(&CellValue::Float(0.1), hex).unwrap().unwrap(),
            "0.1"
        );
        assert_eq!(float_text(1e300), "1e300");
        assert_eq!(float_text(-2.5e-9), "-2.5e-9");
        assert_eq!(float_text(123456.75), "123456.75");
        assert_eq!(float_text(0.0), "0");
        assert_eq!(float_text(f64::NAN), "NaN");
        assert_eq!(
            plain_text(&CellValue::Float(f64::NEG_INFINITY), hex)
                .unwrap()
                .unwrap(),
            "-Infinity"
        );
        assert_eq!(
            plain_text(&CellValue::Bool(true), hex).unwrap().unwrap(),
            "true"
        );
        assert_eq!(
            plain_text(&CellValue::Bytes(vec![1]), BinaryEncoding::Base64)
                .unwrap()
                .unwrap(),
            "AQ=="
        );
        assert_eq!(
            plain_text(&CellValue::Text(String::new()), hex)
                .unwrap()
                .unwrap(),
            ""
        );
        let error = plain_text(&CellValue::unsupported("GEOMETRY"), hex).unwrap_err();
        assert!(error.contains("GEOMETRY"), "{error}");
    }
}
