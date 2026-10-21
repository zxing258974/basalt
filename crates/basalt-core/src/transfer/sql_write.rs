//! SQL INSERT 写器。按连接的方言给标识符加引号、写字面量;每 N 行合成一条 INSERT。
//! 生成列写不进去,跳过;文件一律 UTF-8,开头声明客户端编码。转储(见 [`super::dump`])在 INSERT 前后
//! 另外写建表语句等,由 [`SqlFrame`] 给出。

use std::io::Write;

use super::csv_write::file_error;
use super::render::{float_text, unsupported_reason};
use super::writer::RowWriter;
use super::{ExportColumn, unique_names, value_error};
use crate::error::CoreResult;
use crate::sql::bind::{BindValue, literal, quote_string};
use crate::sql::{Dialect, quote_ident};
use crate::value::{CellValue, format_float};

/// 定点数的文本是不是可以不加引号的数字字面量(PostgreSQL 的 NaN、Infinity 就不是)。
fn is_numeric_literal(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    !unsigned.is_empty()
        && unsigned.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        && unsigned
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
}

/// 一个值在 INSERT 里的字面量写法。
pub fn value_literal(dialect: Dialect, value: &CellValue) -> Result<String, String> {
    Ok(match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        CellValue::Int(n) => n.to_string(),
        CellValue::UInt(n) => n.to_string(),
        CellValue::Float(f) if f.is_finite() => float_text(*f),
        CellValue::Float(f) => match dialect {
            Dialect::Postgres => quote_string(dialect, &format_float(*f)),
            // SQLite 把超出范围的字面量读成无穷大;NaN 存不进 SQLite(会变成 NULL)。
            Dialect::Sqlite if f.is_infinite() => if *f > 0.0 { "9e999" } else { "-9e999" }.into(),
            _ => return Err(format!("是 {}，这种数据库存不了", format_float(*f))),
        },
        CellValue::Decimal(text) if is_numeric_literal(text) => text.clone(),
        CellValue::Bytes(bytes) => match dialect {
            Dialect::Postgres => format!(
                "{}::bytea",
                literal(dialect, &BindValue::Bytes(bytes.clone()))
            ),
            Dialect::Mysql | Dialect::Sqlite => literal(dialect, &BindValue::Bytes(bytes.clone())),
        },
        CellValue::Decimal(text)
        | CellValue::Text(text)
        | CellValue::Json(text)
        | CellValue::Date(text)
        | CellValue::Time(text)
        | CellValue::DateTime(text) => quote_string(dialect, text),
        CellValue::Unsupported { type_name } => return Err(unsupported_reason(type_name)),
    })
}

/// 写在 INSERT 前后的内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlFrame {
    /// 第一行注释的内容。
    pub title: String,
    /// 声明客户端编码那一行之后、第一条 INSERT 之前。
    pub before: String,
    /// 最后一条 INSERT 之后。
    pub after: String,
}

impl Default for SqlFrame {
    fn default() -> Self {
        Self {
            title: "由 Basalt 导出".into(),
            before: String::new(),
            after: String::new(),
        }
    }
}

pub struct SqlWriter<W: Write> {
    out: W,
    dialect: Dialect,
    /// INSERT INTO 后面的表名,已经加好引号。
    target: String,
    frame: SqlFrame,
    rows_per_statement: u64,
    /// 要写的列在行里的位置(跳过生成列)与它们的名字。
    picked: Vec<usize>,
    names: Vec<String>,
    insert_head: String,
    rows: u64,
    buffer: String,
}

impl<W: Write> SqlWriter<W> {
    pub fn new(out: W, dialect: Dialect, table: &str, rows_per_statement: u32) -> Self {
        let target = quote_ident(dialect, table);
        Self::framed(
            out,
            dialect,
            target,
            rows_per_statement,
            SqlFrame::default(),
        )
    }

    /// `target` 是加好引号的表名(可以带 schema)。
    pub fn framed(
        out: W,
        dialect: Dialect,
        target: String,
        rows_per_statement: u32,
        frame: SqlFrame,
    ) -> Self {
        Self {
            out,
            dialect,
            target,
            frame,
            rows_per_statement: u64::from(rows_per_statement.max(1)),
            picked: Vec::new(),
            names: Vec::new(),
            insert_head: String::new(),
            rows: 0,
            buffer: String::new(),
        }
    }

    pub fn into_inner(self) -> W {
        self.out
    }

    fn write(&mut self, text: &str) -> CoreResult<()> {
        self.out.write_all(text.as_bytes()).map_err(file_error)
    }
}

impl<W: Write + Send> RowWriter for SqlWriter<W> {
    fn begin(&mut self, columns: &[ExportColumn]) -> CoreResult<()> {
        let names = unique_names(columns);
        self.picked = (0..columns.len())
            .filter(|&i| !columns[i].is_generated)
            .collect();
        self.names = names;
        let list: Vec<String> = self
            .picked
            .iter()
            .map(|&i| quote_ident(self.dialect, &self.names[i]))
            .collect();
        self.insert_head = format!(
            "INSERT INTO {} ({}) VALUES\n  ",
            self.target,
            list.join(", ")
        );
        // MySQL 和 mysqldump 一样先把 sql_mode 固定下来(结尾再换回去):自增列的 0 原样写进去、不被换成新编号;
        // 反斜杠按转义符算(字面量就是这么写的,不怕执行的连接开着 NO_BACKSLASH_ESCAPES);也不开严格模式,零日期能写回去。
        let encoding = match self.dialect {
            Dialect::Mysql => {
                "SET NAMES utf8mb4;\nSET @OLD_SQL_MODE = @@SQL_MODE, SQL_MODE = 'NO_AUTO_VALUE_ON_ZERO';\n\n"
            }
            Dialect::Postgres => "SET client_encoding = 'UTF8';\n\n",
            Dialect::Sqlite => "\n",
        };
        let preamble = format!("-- {}\n{encoding}{}", self.frame.title, self.frame.before);
        self.write(&preamble)
    }

    fn write_row(&mut self, row: &[CellValue]) -> CoreResult<()> {
        let mut text = std::mem::take(&mut self.buffer);
        text.clear();
        if self.rows.is_multiple_of(self.rows_per_statement) {
            text.push_str(&self.insert_head);
        } else {
            text.push_str(",\n  ");
        }
        self.rows += 1;
        text.push('(');
        for (n, &index) in self.picked.iter().enumerate() {
            if n > 0 {
                text.push_str(", ");
            }
            let literal = value_literal(self.dialect, &row[index])
                .map_err(|reason| value_error(self.rows, &self.names[index], &reason))?;
            text.push_str(&literal);
        }
        text.push(')');
        if self.rows.is_multiple_of(self.rows_per_statement) {
            text.push_str(";\n");
        }
        let result = self.write(&text);
        self.buffer = text;
        result
    }

    fn finish(&mut self) -> CoreResult<()> {
        if !self.rows.is_multiple_of(self.rows_per_statement) {
            self.write(";\n")?;
        }
        if !self.frame.after.is_empty() {
            let after = format!("\n{}", self.frame.after);
            self.write(&after)?;
        }
        if self.dialect == Dialect::Mysql {
            self.write("\nSET SQL_MODE = @OLD_SQL_MODE;\n")?;
        }
        self.out.flush().map_err(file_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(
        dialect: Dialect,
        per: u32,
        columns: &[ExportColumn],
        rows: &[Vec<CellValue>],
    ) -> String {
        let mut writer = SqlWriter::new(Vec::new(), dialect, "用户", per);
        writer.begin(columns).unwrap();
        for row in rows {
            writer.write_row(row).unwrap();
        }
        writer.finish().unwrap();
        String::from_utf8(writer.into_inner()).unwrap()
    }

    #[test]
    fn groups_rows_into_statements_and_skips_generated_columns() {
        let columns = [
            ExportColumn::new("id", "int"),
            ExportColumn {
                is_generated: true,
                ..ExportColumn::new("full", "varchar")
            },
            ExportColumn::new("name", "varchar"),
        ];
        let row = |id: i64, name: &str| {
            vec![
                CellValue::Int(id),
                CellValue::Text("x".into()),
                CellValue::Text(name.into()),
            ]
        };
        let text = export(
            Dialect::Mysql,
            2,
            &columns,
            &[row(1, "O'Brien \\"), row(2, "张三"), row(3, "李四")],
        );
        assert_eq!(
            text,
            "-- 由 Basalt 导出\nSET NAMES utf8mb4;\n\
             SET @OLD_SQL_MODE = @@SQL_MODE, SQL_MODE = 'NO_AUTO_VALUE_ON_ZERO';\n\n\
             INSERT INTO `用户` (`id`, `name`) VALUES\n  (1, 'O''Brien \\\\'),\n  (2, '张三');\n\
             INSERT INTO `用户` (`id`, `name`) VALUES\n  (3, '李四');\n\n\
             SET SQL_MODE = @OLD_SQL_MODE;\n"
        );
        let exact = export(
            Dialect::Sqlite,
            1,
            &columns[..1],
            &[vec![CellValue::Int(1)]],
        );
        assert!(exact.ends_with("VALUES\n  (1);\n"), "{exact}");
        let empty = export(Dialect::Sqlite, 10, &columns, &[]);
        assert_eq!(empty, "-- 由 Basalt 导出\n\n");
    }

    #[test]
    fn a_frame_wraps_the_inserts() {
        let frame = SqlFrame {
            title: "由 Basalt 转储".into(),
            before: "CREATE TABLE t (id int);\n\n".into(),
            after: "SELECT 1;\n".into(),
        };
        let mut writer = SqlWriter::framed(
            Vec::new(),
            Dialect::Postgres,
            "\"s\".\"t\"".into(),
            10,
            frame,
        );
        writer.begin(&[ExportColumn::new("id", "int")]).unwrap();
        writer.write_row(&[CellValue::Int(7)]).unwrap();
        writer.finish().unwrap();
        assert_eq!(
            String::from_utf8(writer.into_inner()).unwrap(),
            "-- 由 Basalt 转储\nSET client_encoding = 'UTF8';\n\nCREATE TABLE t (id int);\n\n\
             INSERT INTO \"s\".\"t\" (\"id\") VALUES\n  (7);\n\nSELECT 1;\n"
        );
    }

    #[test]
    fn literals_follow_each_dialect() {
        let bytes = CellValue::Bytes(vec![0xde, 0xad]);
        assert_eq!(value_literal(Dialect::Mysql, &bytes).unwrap(), "X'dead'");
        assert_eq!(
            value_literal(Dialect::Postgres, &bytes).unwrap(),
            "'\\xdead'::bytea"
        );
        assert_eq!(
            value_literal(Dialect::Postgres, &CellValue::Float(f64::NAN)).unwrap(),
            "'NaN'"
        );
        assert_eq!(
            value_literal(Dialect::Sqlite, &CellValue::Float(f64::INFINITY)).unwrap(),
            "9e999"
        );
        assert!(value_literal(Dialect::Mysql, &CellValue::Float(f64::NAN)).is_err());
        assert_eq!(
            value_literal(Dialect::Mysql, &CellValue::Float(1e300)).unwrap(),
            "1e300"
        );
        assert_eq!(
            value_literal(Dialect::Mysql, &CellValue::Float(0.1)).unwrap(),
            "0.1"
        );
        assert_eq!(
            value_literal(Dialect::Postgres, &CellValue::Decimal("NaN".into())).unwrap(),
            "'NaN'"
        );
        assert_eq!(
            value_literal(
                Dialect::Postgres,
                &CellValue::Decimal("-123456789012345678901234567890.5".into())
            )
            .unwrap(),
            "-123456789012345678901234567890.5"
        );
        assert_eq!(
            value_literal(Dialect::Sqlite, &CellValue::UInt(u64::MAX)).unwrap(),
            "18446744073709551615"
        );
        assert_eq!(
            value_literal(Dialect::Postgres, &CellValue::Bool(false)).unwrap(),
            "FALSE"
        );
        assert_eq!(
            value_literal(Dialect::Postgres, &CellValue::Json("{\"a\":\"'\"}".into())).unwrap(),
            "'{\"a\":\"''\"}'"
        );
        assert!(value_literal(Dialect::Mysql, &CellValue::unsupported("GEOMETRY")).is_err());
    }

    #[test]
    fn numeric_literal_detection() {
        for ok in ["1", "-2.50", "+.5", "1e10", "3.0E-2"] {
            assert!(is_numeric_literal(ok), "{ok}");
        }
        for bad in ["", "NaN", "Infinity", "-", "1;DROP", "e5"] {
            assert!(!is_numeric_literal(bad), "{bad}");
        }
    }
}
