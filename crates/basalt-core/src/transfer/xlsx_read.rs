//! 读 Excel。xlsx 用 calamine 的单元格流式读取,一行一行交出去,不把整张表读进内存;
//! xls、xlsb、ods 没有流式接口,整张表读进来再逐行交出去。

use std::fs::File;
use std::io::BufReader;

use calamine::{Data, DataRef, ExcelDateTime, Reader, Sheets, open_workbook_auto};

use super::source::{SourceRecord, SourceValue};
use crate::error::{CoreError, CoreResult};

fn open(path: &str) -> CoreResult<Sheets<BufReader<File>>> {
    open_workbook_auto(path)
        .map_err(|error| CoreError::File(format!("打不开这个 Excel 文件：{error}")))
}

fn read_error(error: impl std::fmt::Display) -> CoreError {
    CoreError::File(format!("读 Excel 文件出错：{error}"))
}

pub fn sheet_names(path: &str) -> CoreResult<Vec<String>> {
    Ok(open(path)?.sheet_names())
}

/// 日期按 Excel 的格式认出来;[h]:mm:ss 这类时长当文本。
fn datetime(value: &ExcelDateTime) -> SourceValue {
    if value.is_datetime()
        && let Some(datetime) = value.as_datetime()
    {
        return SourceValue::DateTime(datetime);
    }
    match value.as_duration() {
        Some(duration) => {
            let millis = duration.num_milliseconds();
            let (sign, millis) = if millis < 0 {
                ("-", -millis)
            } else {
                ("", millis)
            };
            let seconds = millis / 1000;
            let mut text = format!(
                "{sign}{:02}:{:02}:{:02}",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            );
            if millis % 1000 != 0 {
                text.push_str(format!(".{:03}", millis % 1000).trim_end_matches('0'));
            }
            SourceValue::Text(text)
        }
        None => SourceValue::Number(value.as_f64()),
    }
}

fn from_ref(value: &DataRef<'_>) -> SourceValue {
    match value {
        DataRef::Empty => SourceValue::Null,
        DataRef::Int(n) => SourceValue::Integer(*n),
        DataRef::Float(f) => SourceValue::Number(*f),
        DataRef::String(s) => SourceValue::Text(s.clone()),
        DataRef::SharedString(s) => SourceValue::Text((*s).to_owned()),
        DataRef::Bool(b) => SourceValue::Bool(*b),
        DataRef::DateTime(d) => datetime(d),
        DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => SourceValue::Text(s.clone()),
        DataRef::Error(e) => SourceValue::Error(e.to_string()),
    }
}

fn from_data(value: &Data) -> SourceValue {
    match value {
        Data::Empty => SourceValue::Null,
        Data::Int(n) => SourceValue::Integer(*n),
        Data::Float(f) => SourceValue::Number(*f),
        Data::String(s) => SourceValue::Text(s.clone()),
        Data::Bool(b) => SourceValue::Bool(*b),
        Data::DateTime(d) => datetime(d),
        Data::DateTimeIso(s) | Data::DurationIso(s) => SourceValue::Text(s.clone()),
        Data::Error(e) => SourceValue::Error(e.to_string()),
    }
}

/// 逐行读一个工作表。行号是 Excel 里的行号(从 1 开始);中间整行空着的行不交出去。
pub fn read_sheet(
    path: &str,
    sheet: &str,
    visit: &mut dyn FnMut(SourceRecord) -> CoreResult<bool>,
) -> CoreResult<()> {
    match open(path)? {
        Sheets::Xlsx(mut workbook) => {
            let mut cells = workbook.worksheet_cells_reader(sheet).map_err(read_error)?;
            let mut current: Option<SourceRecord> = None;
            while let Some(cell) = cells.next_cell().map_err(read_error)? {
                let (row, col) = cell.get_position();
                let row = u64::from(row) + 1;
                if current.as_ref().is_some_and(|r| r.row != row)
                    && let Some(done) = current.take()
                    && !visit(done)?
                {
                    return Ok(());
                }
                let record = current.get_or_insert_with(|| SourceRecord {
                    row,
                    values: Vec::new(),
                });
                let col = usize::try_from(col).unwrap_or(usize::MAX);
                if record.values.len() <= col {
                    record.values.resize(col + 1, SourceValue::Null);
                }
                record.values[col] = from_ref(cell.get_value());
            }
            if let Some(done) = current {
                visit(done)?;
            }
            Ok(())
        }
        mut other => {
            let range = other.worksheet_range(sheet).map_err(read_error)?;
            let (first_row, first_col) = range.start().unwrap_or((0, 0));
            for (offset, cells) in range.rows().enumerate() {
                let mut values = vec![SourceValue::Null; first_col as usize];
                values.extend(cells.iter().map(from_data));
                // 电子表格里整行空着就是没有这一行(xlsx 流式读取本来就不会给出空行)。
                if values.iter().all(|v| *v == SourceValue::Null) {
                    continue;
                }
                let record = SourceRecord {
                    row: u64::from(first_row) + offset as u64 + 1,
                    values,
                };
                if !visit(record)? {
                    break;
                }
            }
            Ok(())
        }
    }
}
