//! 四种格式的写器统一成 [`RowWriter`];输出先写到同目录的临时文件,写完才改名成目标文件,
//! 出错或取消时删掉临时文件——不留半个文件,也不碰原来同名的文件。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use super::csv_write::{CsvWriter, file_error};
use super::json_write::JsonWriter;
use super::sql_write::SqlWriter;
use super::xlsx_write::XlsxWriter;
use super::{ExportColumn, ExportFormat, ExportOptions};
use crate::error::{CoreError, CoreResult};
use crate::sql::Dialect;
use crate::value::CellValue;

/// 写文件的缓冲大小。
const BUFFER_BYTES: usize = 256 * 1024;

pub trait RowWriter: Send {
    /// 写文件头(列名行、BOM、开头的说明……)。
    fn begin(&mut self, columns: &[ExportColumn]) -> CoreResult<()>;
    fn write_row(&mut self, row: &[CellValue]) -> CoreResult<()>;
    /// 写文件尾并把缓冲都落盘。
    fn finish(&mut self) -> CoreResult<()>;
}

/// 正在写的输出文件。不调用 [`Self::commit`] 就被丢掉时,临时文件随之删除。
pub struct OutputFile {
    target: PathBuf,
    temp: PathBuf,
    committed: bool,
}

impl OutputFile {
    pub fn new(target: &Path) -> CoreResult<Self> {
        let name = target
            .file_name()
            .ok_or_else(|| CoreError::File("没有指定文件名".into()))?
            .to_string_lossy();
        let temp = target.with_file_name(format!(".{name}.basalt-part"));
        Ok(Self {
            target: target.to_path_buf(),
            temp,
            committed: false,
        })
    }

    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    pub fn create(&self) -> CoreResult<File> {
        File::create(&self.temp).map_err(file_error)
    }

    /// 改名成目标文件(覆盖同名文件),返回文件大小。
    pub fn commit(mut self) -> CoreResult<u64> {
        std::fs::rename(&self.temp, &self.target)
            .map_err(|error| replace_error(error, &self.target))?;
        self.committed = true;
        let size = std::fs::metadata(&self.target).map_err(file_error)?.len();
        Ok(size)
    }
}

/// 覆盖不了目标文件:Windows 上多半是它正在 Excel 之类的程序里开着。
fn replace_error(error: std::io::Error, target: &Path) -> CoreError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return CoreError::File(format!(
            "没能写入 {}：文件可能正被别的程序（比如 Excel）打开，或者没有写权限。关掉它或换个文件名再试。",
            target.display()
        ));
    }
    file_error(error)
}

impl Drop for OutputFile {
    fn drop(&mut self) {
        if !self.committed
            && let Err(error) = std::fs::remove_file(&self.temp)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("删除没写完的导出文件失败：{error}");
        }
    }
}

/// 按格式建写器,写到 `output` 的临时文件里。`sheet` 是 Excel 工作表名的来源。
pub fn open_writer(
    output: &OutputFile,
    options: &ExportOptions,
    dialect: Dialect,
    sheet: &str,
) -> CoreResult<Box<dyn RowWriter>> {
    let binary = options.binary;
    if options.format == ExportFormat::Xlsx {
        return Ok(Box::new(XlsxWriter::new(
            output.temp_path().to_path_buf(),
            sheet,
            binary,
        )));
    }
    let file = BufWriter::with_capacity(BUFFER_BYTES, output.create()?);
    Ok(match &options.format {
        ExportFormat::Csv(csv) => Box::new(CsvWriter::new(file, csv.clone(), binary)),
        ExportFormat::Json { lines } => Box::new(JsonWriter::new(file, *lines, binary)),
        ExportFormat::Sql {
            table,
            rows_per_statement,
        } => Box::new(SqlWriter::new(file, dialect, table, *rows_per_statement)),
        ExportFormat::Xlsx => unreachable!("上面已经处理"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::CsvOptions;

    fn write_all(path: &Path, options: &ExportOptions) -> CoreResult<u64> {
        let output = OutputFile::new(path)?;
        let mut writer = open_writer(&output, options, Dialect::Sqlite, "表")?;
        writer.begin(&[ExportColumn::new("id", "integer")])?;
        writer.write_row(&[CellValue::Int(1)])?;
        writer.finish()?;
        drop(writer);
        output.commit()
    }

    #[test]
    fn writes_through_a_temp_file_and_renames_at_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        std::fs::write(&path, "旧内容").unwrap();
        let options = ExportOptions {
            format: ExportFormat::Csv(CsvOptions::default()),
            binary: Default::default(),
        };
        let size = write_all(&path, &options).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "id\r\n1\r\n");
        assert_eq!(size, 7);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "临时文件已改名"
        );
    }

    #[test]
    fn abandoned_output_removes_the_temp_file_and_keeps_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        std::fs::write(&path, "旧内容").unwrap();
        {
            let output = OutputFile::new(&path).unwrap();
            let options = ExportOptions {
                format: ExportFormat::Json { lines: true },
                binary: Default::default(),
            };
            let mut writer = open_writer(&output, &options, Dialect::Mysql, "t").unwrap();
            writer.begin(&[ExportColumn::new("a", "")]).unwrap();
            assert!(output.temp_path().exists());
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "旧内容");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn every_format_produces_a_file() {
        let dir = tempfile::tempdir().unwrap();
        for (name, format) in [
            ("a.xlsx", ExportFormat::Xlsx),
            ("a.json", ExportFormat::Json { lines: false }),
            (
                "a.sql",
                ExportFormat::Sql {
                    table: "t".into(),
                    rows_per_statement: 100,
                },
            ),
        ] {
            let options = ExportOptions {
                format,
                binary: Default::default(),
            };
            let size = write_all(&dir.path().join(name), &options).unwrap();
            assert!(size > 0, "{name}");
        }
    }

    #[test]
    fn explains_a_target_that_is_open_elsewhere() {
        let target = Path::new("订单.xlsx");
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let message = replace_error(denied, target).to_string();
        assert!(
            message.contains("订单.xlsx") && message.contains("Excel"),
            "{message}"
        );
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(!replace_error(missing, target).to_string().contains("Excel"));
    }
}
