//! Excel(xlsx)写器,用 rust_xlsxwriter 的常量内存模式:每写完一行就落到临时文件,
//! 行数再多内存也不涨。
//!
//! Excel 的数字是双精度浮点、只有 15 位有效数字,所以超过 15 位的整数、定点数写成文本;
//! 日期、日期时间能精确表示时写成真正的 Excel 日期(带格式),带时区、超过毫秒精度、
//! 1900-03-01 之前或不存在的日期写成文本;空字符串写成公式 `=""`,和 NULL(空单元格)分开。
//! 一张表超过 Excel 的行数上限时接着写到下一个工作表。

use std::path::PathBuf;

use rust_xlsxwriter::{ExcelDateTime, Format, Formula, Workbook, XlsxError};

use super::render::{binary_text, unsupported_reason};
use super::temporal::excel_parts;
use super::writer::RowWriter;
use super::{BinaryEncoding, ExportColumn, value_error};
use crate::error::{CoreError, CoreResult};
use crate::value::{CellValue, format_float};

/// Excel 一个工作表最多 1048576 行,第一行是列名。
const MAX_DATA_ROWS: u32 = 1_048_575;
/// Excel 能精确保存的有效数字位数。
const EXACT_DIGITS: usize = 15;
const MAX_SHEET_NAME: usize = 31;
/// 一个单元格最多 32767 个字符。
const MAX_CELL_CHARS: usize = 32_767;

/// 工作表名不能有 `[]:*?/\`,最长 31 个字符。
pub fn sheet_name(base: &str, part: usize) -> String {
    let clean: String = base
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .collect();
    let clean = clean.trim_matches('\'').trim();
    let clean = if clean.is_empty() { "Sheet" } else { clean };
    let suffix = if part > 1 {
        format!(" ({part})")
    } else {
        String::new()
    };
    let room = MAX_SHEET_NAME - suffix.chars().count();
    format!("{}{suffix}", clean.chars().take(room).collect::<String>())
}

/// 十进制文本的有效数字位数;不是普通十进制数时返回 `None`。
fn significant_digits(text: &str) -> Option<usize> {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let trimmed = digits.trim_start_matches('0').trim_end_matches('0');
    Some(trimmed.len())
}

/// 一个值怎么写进单元格。
#[derive(Debug, PartialEq)]
pub enum XlsxCell {
    Blank,
    Number(f64),
    Text(String),
    /// 空字符串:写成公式 `=""`,读回来是空字符串而不是空单元格。
    EmptyText,
    Bool(bool),
    /// 年、月、日、时、分、秒、毫秒。
    Date(DateParts),
    DateTime(DateParts),
}

pub type DateParts = (u16, u8, u8, u16, u8, u8, u16);

fn excel_datetime((y, m, d, h, min, s, ms): DateParts) -> Result<ExcelDateTime, XlsxError> {
    ExcelDateTime::from_ymd(y, m, d)?.and_hms_milli(h, min, s, ms)
}

/// 值 → 单元格。数字超出 Excel 精度、日期 Excel 表示不了时退成文本,不失真。
pub fn cell_for(value: &CellValue, binary: BinaryEncoding) -> Result<XlsxCell, String> {
    let text = |t: String| {
        if t.is_empty() {
            Ok(XlsxCell::EmptyText)
        } else if t.chars().count() > MAX_CELL_CHARS {
            Err(format!(
                "超过 Excel 单元格上限（{MAX_CELL_CHARS} 个字符），请改用 CSV 导出"
            ))
        } else {
            Ok(XlsxCell::Text(t))
        }
    };
    let exact = |digits: String| -> Result<XlsxCell, String> {
        match (significant_digits(&digits), digits.parse::<f64>()) {
            (Some(n), Ok(number)) if n <= EXACT_DIGITS => Ok(XlsxCell::Number(number)),
            _ => text(digits),
        }
    };
    match value {
        CellValue::Null => Ok(XlsxCell::Blank),
        CellValue::Bool(b) => Ok(XlsxCell::Bool(*b)),
        CellValue::Int(n) => exact(n.to_string()),
        CellValue::UInt(n) => exact(n.to_string()),
        CellValue::Float(f) if f.is_finite() => Ok(XlsxCell::Number(*f)),
        CellValue::Float(f) => text(format_float(*f)),
        CellValue::Decimal(digits) => exact(digits.clone()),
        CellValue::Date(t) => match excel_parts(t, false) {
            Some(parts) => Ok(XlsxCell::Date(parts)),
            None => text(t.clone()),
        },
        CellValue::DateTime(t) => match excel_parts(t, true) {
            Some(parts) => Ok(XlsxCell::DateTime(parts)),
            None => text(t.clone()),
        },
        CellValue::Bytes(bytes) => text(binary_text(bytes, binary)),
        CellValue::Text(t) | CellValue::Json(t) | CellValue::Time(t) => text(t.clone()),
        CellValue::Unsupported { type_name } => Err(unsupported_reason(type_name)),
    }
}

fn xlsx_error(error: XlsxError) -> CoreError {
    CoreError::File(format!("写 Excel 文件出错：{error}"))
}

pub struct XlsxWriter {
    workbook: Workbook,
    path: PathBuf,
    base_name: String,
    binary: BinaryEncoding,
    names: Vec<String>,
    sheet: usize,
    row_in_sheet: u32,
    rows_per_sheet: u32,
    rows: u64,
    header: Format,
    date: Format,
    datetime: Format,
    datetime_ms: Format,
}

impl XlsxWriter {
    /// `path` 是最后保存到的文件;`sheet` 是工作表名的来源(表名)。
    pub fn new(path: PathBuf, sheet: &str, binary: BinaryEncoding) -> Self {
        Self {
            workbook: Workbook::new(),
            path,
            base_name: sheet.to_owned(),
            binary,
            names: Vec::new(),
            sheet: 0,
            row_in_sheet: 0,
            rows_per_sheet: MAX_DATA_ROWS,
            rows: 0,
            header: Format::new().set_bold(),
            date: Format::new().set_num_format("yyyy-mm-dd"),
            datetime: Format::new().set_num_format("yyyy-mm-dd hh:mm:ss"),
            datetime_ms: Format::new().set_num_format("yyyy-mm-dd hh:mm:ss.000"),
        }
    }

    /// 测试用:不用真写一百万行就能测换工作表。
    #[cfg(test)]
    fn with_rows_per_sheet(mut self, rows: u32) -> Self {
        self.rows_per_sheet = rows;
        self
    }

    /// 开一个新的工作表并写好列名行。
    fn add_sheet(&mut self) -> CoreResult<()> {
        let part = self.workbook.worksheets().len() + 1;
        let sheet = self
            .workbook
            .add_worksheet_with_constant_memory()
            .map_err(xlsx_error)?;
        sheet
            .set_name(sheet_name(&self.base_name, part))
            .map_err(xlsx_error)?;
        sheet.set_freeze_panes(1, 0).map_err(xlsx_error)?;
        // 只写 `=""` 这一种公式:结果留空,读回来才是空字符串(默认结果是 0)。
        sheet.set_formula_result_default("");
        for (index, name) in self.names.iter().enumerate() {
            let col = u16::try_from(index).map_err(|_| too_many_columns())?;
            let width = name.chars().count().clamp(10, 40);
            sheet
                .set_column_width(col, f64::from(u32::try_from(width).unwrap_or(10)))
                .map_err(xlsx_error)?;
            sheet
                .write_string_with_format(0, col, name, &self.header)
                .map_err(xlsx_error)?;
        }
        self.sheet = part - 1;
        self.row_in_sheet = 0;
        Ok(())
    }
}

fn too_many_columns() -> CoreError {
    CoreError::InvalidData("列数超过了 Excel 的上限（16384 列）".into())
}

impl RowWriter for XlsxWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> CoreResult<()> {
        if columns.len() > 16_384 {
            return Err(too_many_columns());
        }
        self.names = columns.iter().map(|c| c.name.clone()).collect();
        self.add_sheet()
    }

    fn write_row(&mut self, row: &[CellValue]) -> CoreResult<()> {
        if self.row_in_sheet == self.rows_per_sheet {
            self.add_sheet()?;
        }
        self.rows += 1;
        self.row_in_sheet += 1;
        let (number, line) = (self.rows, self.row_in_sheet);
        let sheet = self
            .workbook
            .worksheet_from_index(self.sheet)
            .map_err(xlsx_error)?;
        for (index, value) in row.iter().enumerate() {
            let col = u16::try_from(index).map_err(|_| too_many_columns())?;
            let cell = cell_for(value, self.binary)
                .map_err(|reason| value_error(number, &self.names[index], &reason))?;
            let written = match cell {
                XlsxCell::Blank => continue,
                XlsxCell::Number(n) => sheet.write_number(line, col, n),
                XlsxCell::Text(t) => sheet.write_string(line, col, t),
                XlsxCell::EmptyText => sheet.write_formula(line, col, Formula::new("=\"\"")),
                XlsxCell::Bool(b) => sheet.write_boolean(line, col, b),
                XlsxCell::Date(parts) => excel_datetime(parts)
                    .and_then(|d| sheet.write_datetime_with_format(line, col, &d, &self.date)),
                XlsxCell::DateTime(parts) => {
                    let format = if parts.6 == 0 {
                        &self.datetime
                    } else {
                        &self.datetime_ms
                    };
                    excel_datetime(parts)
                        .and_then(|d| sheet.write_datetime_with_format(line, col, &d, format))
                }
            };
            written.map_err(xlsx_error)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> CoreResult<()> {
        self.workbook.save(&self.path).map_err(xlsx_error)
    }
}

#[cfg(test)]
#[path = "xlsx_write_tests.rs"]
mod tests;
