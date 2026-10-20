//! 导入的来源文件:CSV 或 Excel。统一成一行一行的 [`SourceValue`],带上它在文件里的行号。

use std::fs::File;
use std::io::Read;
use std::path::Path;

use chrono::{NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};

use super::TextEncoding;
use super::csv_read::{CsvParser, CsvRecord, detect_delimiter};
use super::csv_write::file_error;
use super::encoding::{Decoder, detect};
use super::temporal::{format_datetime, format_time};
use super::xlsx_read;
use crate::error::{CoreError, CoreResult};
use crate::value::format_float;

/// 预览给多少行数据。
pub const PREVIEW_ROWS: usize = 20;
/// 读 CSV 的块大小,也是探测编码和分隔符的样本大小。
const CHUNK_BYTES: usize = 64 * 1024;

/// 来源里的一个值。CSV 只有文本和 NULL;Excel 还有数字、布尔、日期和错误值。
#[derive(Clone, Debug, PartialEq)]
pub enum SourceValue {
    /// CSV 里没加引号的空字段、Excel 的空单元格。
    Null,
    Text(String),
    Number(f64),
    Integer(i64),
    Bool(bool),
    DateTime(NaiveDateTime),
    /// Excel 的错误值,如 #N/A。
    Error(String),
}

impl SourceValue {
    /// 给人看的文本(预览、失败报告);NULL 为 `None`。
    pub fn display(&self) -> Option<String> {
        Some(match self {
            Self::Null => return None,
            Self::Text(text) => text.clone(),
            Self::Number(number) => format_float(*number),
            Self::Integer(number) => number.to_string(),
            Self::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
            Self::DateTime(value) => describe_datetime(*value),
            Self::Error(error) => error.clone(),
        })
    }
}

/// Excel 的纯时间挂在 1899-12-30 上;零点的日期只显示日期。
pub fn describe_datetime(value: NaiveDateTime) -> String {
    let date = value.date().to_string();
    if date == "1899-12-30" || date == "1899-12-31" {
        format_time(value.time())
    } else if value.num_seconds_from_midnight() == 0 && value.nanosecond() == 0 {
        date
    } else {
        format_datetime(value)
    }
}

/// 一行数据。`row` 是它在文件里的行号(CSV 的第几条记录、Excel 的行号),失败报告里用。
#[derive(Clone, Debug, PartialEq)]
pub struct SourceRecord {
    pub row: u64,
    pub values: Vec<SourceValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SourceFormat {
    /// `encoding`、`delimiter` 不给就自动探测。
    Csv {
        #[serde(default)]
        encoding: Option<TextEncoding>,
        #[serde(default)]
        delimiter: Option<char>,
        quote: char,
    },
    /// `sheet` 不给就用第一个工作表。
    Excel {
        #[serde(default)]
        sheet: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpec {
    pub path: String,
    /// 不给就按扩展名判断。
    #[serde(default)]
    pub format: Option<SourceFormat>,
    pub has_header: bool,
}

/// 文件的预览:实际用的格式设置、列名和前几行。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreview {
    /// 探测之后实际用的格式(编码、分隔符、工作表都填好了)。
    pub format: SourceFormat,
    /// Excel 的全部工作表名;CSV 为空。
    pub sheets: Vec<String>,
    pub headers: Vec<String>,
    /// 前几行,`None` 是 NULL。
    pub rows: Vec<Vec<Option<String>>>,
    /// 这几行在文件里的行号。
    pub row_numbers: Vec<u64>,
}

/// 按扩展名判断格式。
pub fn format_for_path(path: &str) -> CoreResult<SourceFormat> {
    let extension = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "csv" | "tsv" | "txt" => Ok(SourceFormat::Csv {
            encoding: None,
            delimiter: (extension == "tsv").then_some('\t'),
            quote: '"',
        }),
        "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" => Ok(SourceFormat::Excel { sheet: None }),
        _ => Err(CoreError::Unsupported(
            "只能导入 CSV（.csv、.tsv、.txt）和 Excel（.xlsx、.xls）文件。".into(),
        )),
    }
}

/// 把格式里没定下来的(编码、分隔符、工作表)按文件内容定下来。
pub fn resolve_format(spec: &SourceSpec) -> CoreResult<(SourceFormat, Vec<String>)> {
    let format = match &spec.format {
        Some(format) => format.clone(),
        None => format_for_path(&spec.path)?,
    };
    match format {
        SourceFormat::Csv {
            encoding,
            delimiter,
            quote,
        } => {
            let sample = read_sample(&spec.path)?;
            let encoding = encoding.unwrap_or_else(|| detect(&sample));
            let delimiter = match delimiter {
                Some(delimiter) => delimiter,
                None => {
                    let mut text = String::new();
                    // 样本可能截在半个字符上、也可能根本不是这个编码,探测分隔符时不计较。
                    let _ = Decoder::new(encoding).decode(&sample, false, &mut text);
                    detect_delimiter(&text, quote)
                }
            };
            let format = SourceFormat::Csv {
                encoding: Some(encoding),
                delimiter: Some(delimiter),
                quote,
            };
            Ok((format, Vec::new()))
        }
        SourceFormat::Excel { sheet } => {
            let sheets = xlsx_read::sheet_names(&spec.path)?;
            let sheet = match sheet {
                Some(sheet) if sheets.contains(&sheet) => sheet,
                Some(sheet) => {
                    return Err(CoreError::NotFound(format!(
                        "文件里没有工作表「{sheet}」。"
                    )));
                }
                None => sheets.first().cloned().ok_or_else(|| {
                    CoreError::InvalidData("这个 Excel 文件里没有工作表。".into())
                })?,
            };
            Ok((SourceFormat::Excel { sheet: Some(sheet) }, sheets))
        }
    }
}

fn read_sample(path: &str) -> CoreResult<Vec<u8>> {
    let mut sample = Vec::with_capacity(CHUNK_BYTES);
    File::open(path)
        .map_err(file_error)?
        .take(CHUNK_BYTES as u64)
        .read_to_end(&mut sample)
        .map_err(file_error)?;
    Ok(sample)
}

/// 没有列名行或列名是空的:按位置叫「第 N 列」。
pub fn column_label(index: usize) -> String {
    format!("第 {} 列", index + 1)
}

/// 读出列名与前 [`PREVIEW_ROWS`] 行。
pub fn inspect(spec: &SourceSpec) -> CoreResult<SourcePreview> {
    let (format, sheets) = resolve_format(spec)?;
    let resolved = SourceSpec {
        format: Some(format.clone()),
        ..spec.clone()
    };
    let mut headers: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    let mut row_numbers = Vec::new();
    read_rows(
        &resolved,
        |names| headers = names,
        |record| {
            rows.push(record.values.iter().map(SourceValue::display).collect());
            row_numbers.push(record.row);
            Ok(rows.len() < PREVIEW_ROWS)
        },
    )?;
    if !spec.has_header {
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        headers = (0..width).map(column_label).collect();
    }
    Ok(SourcePreview {
        format,
        sheets,
        headers,
        rows,
        row_numbers,
    })
}

/// 读整份文件。有列名行时先把列名交给 `on_headers`,数据行逐条交给 `visit`;
/// `visit` 返回 false 就停止。空行跳过,但行号照算。`spec.format` 必须已经定下来。
pub fn read_rows(
    spec: &SourceSpec,
    mut on_headers: impl FnMut(Vec<String>),
    mut visit: impl FnMut(SourceRecord) -> CoreResult<bool>,
) -> CoreResult<()> {
    let mut header_pending = spec.has_header;
    let mut handle = |record: SourceRecord| -> CoreResult<bool> {
        if record.values.iter().all(|v| *v == SourceValue::Null) {
            return Ok(true);
        }
        if header_pending {
            header_pending = false;
            let names = record
                .values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    value
                        .display()
                        .map(|name| name.trim().to_owned())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| column_label(index))
                })
                .collect();
            on_headers(names);
            return Ok(true);
        }
        visit(record)
    };
    match spec.format.as_ref() {
        Some(SourceFormat::Csv {
            encoding: Some(encoding),
            delimiter: Some(delimiter),
            quote,
        }) => read_csv(&spec.path, *encoding, *delimiter, *quote, &mut handle),
        Some(SourceFormat::Excel { sheet: Some(sheet) }) => {
            xlsx_read::read_sheet(&spec.path, sheet, &mut handle)
        }
        _ => Err(CoreError::InvalidData(
            "导入前要先确定文件的编码、分隔符或工作表。".into(),
        )),
    }
}

fn csv_values(record: CsvRecord) -> SourceRecord {
    SourceRecord {
        row: record.number,
        values: record
            .fields
            .into_iter()
            .map(|field| match (field.text.is_empty(), field.quoted) {
                (true, false) => SourceValue::Null,
                _ => SourceValue::Text(field.text),
            })
            .collect(),
    }
}

fn read_csv(
    path: &str,
    encoding: TextEncoding,
    delimiter: char,
    quote: char,
    visit: &mut dyn FnMut(SourceRecord) -> CoreResult<bool>,
) -> CoreResult<()> {
    let mut file = File::open(path).map_err(file_error)?;
    let mut decoder = Decoder::new(encoding);
    let mut parser = CsvParser::new(delimiter, quote);
    let mut bytes = vec![0; CHUNK_BYTES];
    let mut text = String::new();
    let mut records = Vec::new();
    loop {
        let read = file.read(&mut bytes).map_err(file_error)?;
        let last = read == 0;
        text.clear();
        decoder
            .decode(&bytes[..read], last, &mut text)
            .map_err(CoreError::InvalidData)?;
        parser.feed(&text, &mut records);
        if last {
            parser
                .finish(&mut records)
                .map_err(CoreError::InvalidData)?;
        }
        for record in records.drain(..) {
            if !visit(csv_values(record))? {
                return Ok(());
            }
        }
        if last {
            return Ok(());
        }
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
