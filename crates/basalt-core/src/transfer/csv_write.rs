//! CSV 写器。NULL 写成空字段,空字符串写成一对引号,导入时据此分开两者。
//! 行尾用 CRLF(RFC 4180);字段里有分隔符、引号、换行或首尾空白时加引号,引号成对转义。

use std::io::Write;

use super::encoding::{encode, preamble};
use super::render::plain_text;
use super::writer::RowWriter;
use super::{BinaryEncoding, CsvOptions, ExportColumn, value_error};
use crate::error::{CoreError, CoreResult};
use crate::value::CellValue;

const LINE_END: &str = "\r\n";

fn needs_quotes(text: &str, options: &CsvOptions) -> bool {
    text.is_empty()
        || text.contains([options.delimiter, options.quote, '\r', '\n'])
        || text.starts_with([' ', '\t'])
        || text.ends_with([' ', '\t'])
}

/// 把一个字段追加到 `out`。`None` 是 NULL。
pub fn push_field(out: &mut String, field: Option<&str>, options: &CsvOptions) {
    let Some(text) = field else {
        return;
    };
    if !needs_quotes(text, options) {
        out.push_str(text);
        return;
    }
    let quote = options.quote;
    out.push(quote);
    for c in text.chars() {
        if c == quote {
            out.push(quote);
        }
        out.push(c);
    }
    out.push(quote);
}

pub struct CsvWriter<W: Write> {
    out: W,
    options: CsvOptions,
    binary: BinaryEncoding,
    names: Vec<String>,
    line: String,
    rows: u64,
}

impl<W: Write> CsvWriter<W> {
    pub fn new(out: W, options: CsvOptions, binary: BinaryEncoding) -> Self {
        Self {
            out,
            options,
            binary,
            names: Vec::new(),
            line: String::new(),
            rows: 0,
        }
    }

    pub fn into_inner(self) -> W {
        self.out
    }

    /// 把攒好的一行编码后写出去。编码失败时 `describe` 说清是哪一格。
    fn flush_line(&mut self, describe: impl FnOnce(char) -> CoreError) -> CoreResult<()> {
        self.line.push_str(LINE_END);
        let bytes = encode(self.options.encoding, &self.line).map_err(describe)?;
        self.out.write_all(&bytes).map_err(file_error)?;
        self.line.clear();
        Ok(())
    }
}

pub(crate) fn file_error(error: std::io::Error) -> CoreError {
    CoreError::File(error.to_string())
}

fn gbk_reason(c: char) -> String {
    format!("里的「{c}」GBK 表示不了，请改用 UTF-8 导出")
}

impl<W: Write + Send> RowWriter for CsvWriter<W> {
    fn begin(&mut self, columns: &[ExportColumn]) -> CoreResult<()> {
        self.names = columns.iter().map(|c| c.name.clone()).collect();
        self.out
            .write_all(preamble(self.options.encoding))
            .map_err(file_error)?;
        if !self.options.header {
            return Ok(());
        }
        for (index, column) in columns.iter().enumerate() {
            if index > 0 {
                self.line.push(self.options.delimiter);
            }
            push_field(&mut self.line, Some(&column.name), &self.options);
        }
        let names = self.names.clone();
        self.flush_line(|c| {
            let column = names
                .iter()
                .find(|n| n.contains(c))
                .cloned()
                .unwrap_or_default();
            CoreError::InvalidData(format!("列名「{column}」{}", gbk_reason(c)))
        })
    }

    fn write_row(&mut self, row: &[CellValue]) -> CoreResult<()> {
        self.rows += 1;
        let number = self.rows;
        for (index, (value, name)) in row.iter().zip(&self.names).enumerate() {
            if index > 0 {
                self.line.push(self.options.delimiter);
            }
            let text = plain_text(value, self.binary)
                .map_err(|reason| value_error(number, name, &reason))?;
            push_field(&mut self.line, text.as_deref(), &self.options);
        }
        let binary = self.binary;
        let names = std::mem::take(&mut self.names);
        let result = self.flush_line(|c| {
            // 找出是哪一列有这个字符,报错时指给用户看。
            let column = row
                .iter()
                .zip(&names)
                .find(|(value, _)| {
                    plain_text(value, binary)
                        .ok()
                        .flatten()
                        .is_some_and(|t| t.contains(c))
                })
                .map(|(_, name)| name.as_str())
                .unwrap_or_default();
            value_error(number, column, &gbk_reason(c))
        });
        self.names = names;
        result
    }

    fn finish(&mut self) -> CoreResult<()> {
        self.out.flush().map_err(file_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::TextEncoding;

    fn columns(names: &[&str]) -> Vec<ExportColumn> {
        names.iter().map(|n| ExportColumn::new(n, "")).collect()
    }

    fn write(options: CsvOptions, names: &[&str], rows: &[Vec<CellValue>]) -> CoreResult<Vec<u8>> {
        let columns = columns(names);
        let mut writer = CsvWriter::new(Vec::new(), options, BinaryEncoding::Hex);
        writer.begin(&columns)?;
        for row in rows {
            writer.write_row(row)?;
        }
        writer.finish()?;
        Ok(writer.into_inner())
    }

    #[test]
    fn null_is_an_empty_field_and_empty_string_is_quoted() {
        let bytes = write(
            CsvOptions::default(),
            &["id", "note"],
            &[
                vec![CellValue::Int(1), CellValue::Null],
                vec![CellValue::Int(2), CellValue::Text(String::new())],
            ],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "id,note\r\n1,\r\n2,\"\"\r\n"
        );
    }

    #[test]
    fn quotes_fields_with_special_characters() {
        let bytes = write(
            CsvOptions::default(),
            &["a"],
            &[
                vec![CellValue::Text("x,y".into())],
                vec![CellValue::Text("他说\"好\"".into())],
                vec![CellValue::Text("两\n行".into())],
                vec![CellValue::Text(" 前面有空格".into())],
                vec![CellValue::Text("普通".into())],
            ],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "a\r\n\"x,y\"\r\n\"他说\"\"好\"\"\"\r\n\"两\n行\"\r\n\" 前面有空格\"\r\n普通\r\n"
        );
    }

    #[test]
    fn custom_delimiter_quote_header_and_bom() {
        let options = CsvOptions {
            delimiter: '\t',
            quote: '\'',
            header: false,
            encoding: TextEncoding::Utf8Bom,
        };
        let bytes = write(
            options,
            &["a", "b"],
            &[vec![
                CellValue::Text("it's".into()),
                CellValue::Text("a,b".into()),
            ]],
        )
        .unwrap();
        assert_eq!(bytes, "\u{feff}'it''s'\ta,b\r\n".as_bytes());
    }

    #[test]
    fn gbk_output_and_unmappable_characters_name_the_cell() {
        let options = CsvOptions {
            encoding: TextEncoding::Gbk,
            ..CsvOptions::default()
        };
        let bytes = write(
            options.clone(),
            &["名"],
            &[vec![CellValue::Text("中".into())]],
        )
        .unwrap();
        assert_eq!(bytes, b"\xC3\xFB\r\n\xD6\xD0\r\n");

        let error = write(
            options.clone(),
            &["id", "名"],
            &[vec![CellValue::Int(1), CellValue::Text("笑😀".into())]],
        )
        .unwrap_err();
        assert_eq!(error.code(), "invalid_data");
        let message = error.to_string();
        assert!(
            message.contains("第 1 行「名」") && message.contains("😀"),
            "{message}"
        );

        let error = write(options, &["😀"], &[]).unwrap_err();
        assert!(error.to_string().contains("列名"), "{error}");
    }

    #[test]
    fn unsupported_values_stop_the_export_with_the_cell_named() {
        let error = write(
            CsvOptions::default(),
            &["geom"],
            &[vec![CellValue::unsupported("GEOMETRY")]],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("第 1 行「geom」是 GEOMETRY"),
            "{error}"
        );
    }
}
