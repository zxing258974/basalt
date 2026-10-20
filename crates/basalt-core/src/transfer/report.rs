//! 导入的失败报告:哪一行、哪一列、什么值、为什么。可以另存成 CSV。

use std::io::{BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::CsvOptions;
use super::csv_write::{file_error, push_field};
use super::encoding::UTF8_BOM;
use super::writer::OutputFile;
use crate::error::CoreResult;

/// 报告里最多带回多少条失败明细;再多只计数。
pub const MAX_FAILURES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    /// 文件里的行号(和电子表格里看到的一致)。
    pub row: u64,
    /// 类型转换失败时是哪一列;数据库拒绝整行时为空。
    #[serde(default)]
    pub column: Option<String>,
    /// 出问题的原值;NULL 或整行失败时为空。
    #[serde(default)]
    pub value: Option<String>,
    pub reason: String,
}

/// 收集失败:超过上限的只计数。
#[derive(Debug, Default)]
pub struct Failures {
    pub items: Vec<ImportFailure>,
    pub count: u64,
}

impl Failures {
    pub fn push(&mut self, failure: ImportFailure) {
        self.count += 1;
        if self.items.len() < MAX_FAILURES {
            self.items.push(failure);
        }
    }

    pub fn truncated(&self) -> bool {
        self.count > self.items.len() as u64
    }
}

/// 写成带 BOM 的 UTF-8 CSV(Excel 直接打开不乱码):行号、列、值、原因。
pub fn write_failures_csv(path: &Path, failures: &[ImportFailure]) -> CoreResult<u64> {
    let output = OutputFile::new(path)?;
    let mut out = BufWriter::new(output.create()?);
    let options = CsvOptions::default();
    let mut text = String::new();
    let line = |fields: [Option<&str>; 4], text: &mut String| {
        text.clear();
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                text.push(',');
            }
            push_field(text, *field, &options);
        }
        text.push_str("\r\n");
    };
    out.write_all(UTF8_BOM).map_err(file_error)?;
    line(
        [Some("行号"), Some("列"), Some("值"), Some("原因")],
        &mut text,
    );
    out.write_all(text.as_bytes()).map_err(file_error)?;
    for failure in failures {
        let row = failure.row.to_string();
        line(
            [
                Some(&row),
                failure.column.as_deref(),
                failure.value.as_deref(),
                Some(&failure.reason),
            ],
            &mut text,
        );
        out.write_all(text.as_bytes()).map_err(file_error)?;
    }
    out.flush().map_err(file_error)?;
    drop(out);
    output.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_first_failures_and_counts_the_rest() {
        let mut failures = Failures::default();
        for row in 0..(MAX_FAILURES as u64 + 5) {
            failures.push(ImportFailure {
                row,
                column: None,
                value: None,
                reason: "x".into(),
            });
        }
        assert_eq!(failures.items.len(), MAX_FAILURES);
        assert_eq!(failures.count, MAX_FAILURES as u64 + 5);
        assert!(failures.truncated());
    }

    #[test]
    fn writes_a_csv_excel_can_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("失败.csv");
        write_failures_csv(
            &path,
            &[
                ImportFailure {
                    row: 3,
                    column: Some("年龄".into()),
                    value: Some("4x2".into()),
                    reason: "不是有效的整数".into(),
                },
                ImportFailure {
                    row: 9,
                    column: None,
                    value: None,
                    reason: "Duplicate entry '1' for key 'PRIMARY'".into(),
                },
            ],
        )
        .unwrap();
        let text = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            text,
            "\u{feff}行号,列,值,原因\r\n3,年龄,4x2,不是有效的整数\r\n9,,,Duplicate entry '1' for key 'PRIMARY'\r\n"
        );
    }
}
