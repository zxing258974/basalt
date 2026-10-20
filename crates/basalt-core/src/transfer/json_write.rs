//! JSON 写器:一个数组,或每行一个对象(JSON Lines)。文件一律 UTF-8。
//!
//! 整数写成 JSON 数字,位数原样;定点数写成字符串,保留全部位数和末尾的零(读成浮点数会失真);
//! NaN / Infinity 不是合法的 JSON 数字,写成字符串;JSON 列原样嵌进去;二进制按选项编码成字符串。

use std::io::Write;

use serde::de::IgnoredAny;

use super::csv_write::file_error;
use super::render::{binary_text, float_text, unsupported_reason};
use super::writer::RowWriter;
use super::{BinaryEncoding, ExportColumn, unique_names, value_error};
use crate::error::CoreResult;
use crate::value::{CellValue, format_float};

fn push_string(out: &mut String, text: &str) {
    // serde_json 的字符串转义不会失败。
    out.push_str(&serde_json::to_string(text).unwrap_or_default());
}

/// 一个值的 JSON 写法。
pub fn push_value(
    out: &mut String,
    value: &CellValue,
    binary: BinaryEncoding,
) -> Result<(), String> {
    match value {
        CellValue::Null => out.push_str("null"),
        CellValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        CellValue::Int(n) => out.push_str(&n.to_string()),
        CellValue::UInt(n) => out.push_str(&n.to_string()),
        CellValue::Float(f) if f.is_finite() => out.push_str(&float_text(*f)),
        CellValue::Float(f) => push_string(out, &format_float(*f)),
        CellValue::Json(text) if serde_json::from_str::<IgnoredAny>(text).is_ok() => {
            out.push_str(text.trim());
        }
        CellValue::Bytes(bytes) => push_string(out, &binary_text(bytes, binary)),
        CellValue::Decimal(text)
        | CellValue::Text(text)
        | CellValue::Json(text)
        | CellValue::Date(text)
        | CellValue::Time(text)
        | CellValue::DateTime(text) => push_string(out, text),
        CellValue::Unsupported { type_name } => return Err(unsupported_reason(type_name)),
    }
    Ok(())
}

pub struct JsonWriter<W: Write> {
    out: W,
    lines: bool,
    binary: BinaryEncoding,
    names: Vec<String>,
    /// 每个键预先转义好,写每行时直接拼。
    keys: Vec<String>,
    buffer: String,
    rows: u64,
}

impl<W: Write> JsonWriter<W> {
    pub fn new(out: W, lines: bool, binary: BinaryEncoding) -> Self {
        Self {
            out,
            lines,
            binary,
            names: Vec::new(),
            keys: Vec::new(),
            buffer: String::new(),
            rows: 0,
        }
    }

    pub fn into_inner(self) -> W {
        self.out
    }

    fn write(&mut self, text: &str) -> CoreResult<()> {
        self.out.write_all(text.as_bytes()).map_err(file_error)
    }
}

impl<W: Write + Send> RowWriter for JsonWriter<W> {
    fn begin(&mut self, columns: &[ExportColumn]) -> CoreResult<()> {
        self.names = unique_names(columns);
        self.keys = self
            .names
            .iter()
            .map(|name| {
                let mut key = String::new();
                push_string(&mut key, name);
                key.push(':');
                key
            })
            .collect();
        if self.lines { Ok(()) } else { self.write("[") }
    }

    fn write_row(&mut self, row: &[CellValue]) -> CoreResult<()> {
        self.rows += 1;
        let mut line = std::mem::take(&mut self.buffer);
        line.clear();
        if !self.lines {
            line.push_str(if self.rows == 1 { "\n  " } else { ",\n  " });
        }
        line.push('{');
        for (index, value) in row.iter().enumerate() {
            if index > 0 {
                line.push(',');
            }
            line.push_str(&self.keys[index]);
            push_value(&mut line, value, self.binary)
                .map_err(|reason| value_error(self.rows, &self.names[index], &reason))?;
        }
        line.push('}');
        if self.lines {
            line.push('\n');
        }
        let result = self.write(&line);
        self.buffer = line;
        result
    }

    fn finish(&mut self) -> CoreResult<()> {
        if !self.lines {
            let end = if self.rows == 0 { "]\n" } else { "\n]\n" };
            self.write(end)?;
        }
        self.out.flush().map_err(file_error)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn write(lines: bool, names: &[&str], rows: &[Vec<CellValue>]) -> CoreResult<String> {
        let columns: Vec<ExportColumn> = names.iter().map(|n| ExportColumn::new(n, "")).collect();
        let mut writer = JsonWriter::new(Vec::new(), lines, BinaryEncoding::Hex);
        writer.begin(&columns)?;
        for row in rows {
            writer.write_row(row)?;
        }
        writer.finish()?;
        Ok(String::from_utf8(writer.into_inner()).unwrap())
    }

    #[test]
    fn array_keeps_big_numbers_decimals_and_embedded_json() {
        let text = write(
            false,
            &["id", "price", "ratio", "doc", "名字", "blob", "gone"],
            &[vec![
                CellValue::Int(9_007_199_254_740_993),
                CellValue::Decimal("12345678901234567890.10".into()),
                CellValue::Float(f64::NAN),
                CellValue::Json("{\"a\": [1, 2]}".into()),
                CellValue::Text("张\"三\"\n".into()),
                CellValue::Bytes(vec![0xab]),
                CellValue::Null,
            ]],
        )
        .unwrap();
        assert!(
            text.contains("\"id\":9007199254740993,"),
            "大整数不能变成浮点数：{text}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed,
            json!([{
                "id": 9_007_199_254_740_993_u64,
                "price": "12345678901234567890.10",
                "ratio": "NaN",
                "doc": { "a": [1, 2] },
                "名字": "张\"三\"\n",
                "blob": "0xab",
                "gone": null
            }])
        );
    }

    #[test]
    fn lines_format_and_empty_array() {
        let text = write(
            true,
            &["a", "a"],
            &[
                vec![CellValue::Int(1), CellValue::Bool(true)],
                vec![CellValue::Int(2), CellValue::Bool(false)],
            ],
        )
        .unwrap();
        assert_eq!(
            text,
            "{\"a\":1,\"a (2)\":true}\n{\"a\":2,\"a (2)\":false}\n"
        );
        assert_eq!(write(false, &["a"], &[]).unwrap(), "[]\n");
        let two = write(
            false,
            &["a"],
            &[vec![CellValue::Int(1)], vec![CellValue::Int(2)]],
        )
        .unwrap();
        assert_eq!(two, "[\n  {\"a\":1},\n  {\"a\":2}\n]\n");
    }

    #[test]
    fn invalid_json_text_is_kept_as_a_string_and_unsupported_values_fail() {
        let text = write(false, &["doc"], &[vec![CellValue::Json("{oops".into())]]).unwrap();
        assert!(text.contains("\"doc\":\"{oops\""), "{text}");
        let error = write(true, &["g"], &[vec![CellValue::unsupported("POINT")]]).unwrap_err();
        assert!(error.to_string().contains("第 1 行「g」"), "{error}");
    }
}
