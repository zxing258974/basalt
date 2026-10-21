//! 导入导出(M6)。
//!
//! 导出:表(整张或按筛选条件)、查询结果写成 CSV / Excel / JSON / SQL INSERT。数据从一条专用连接上
//! 流式读出、边读边写,内存不随行数增长;取消走查询标签页同一套机制(见 [`crate::query::lease`])。
//!
//! 转储:一张表的建表语句连同全部数据写成可以直接执行的 SQL 文件(见 [`dump`]),读数据、进度和取消与导出共用。
//!
//! 导入:CSV / Excel 按列映射写进一张表。按目标列的类型先在客户端解析,解析不了的进失败报告,
//! 不静默改成 NULL;一律参数化执行。
//!
//! 写器、解析、映射、类型转换都是纯函数或只碰本地文件,不碰数据库;和数据库打交道的编排在
//! [`export`] 与 [`import`] 里。

pub mod convert;
mod convert_temporal;
pub mod csv_read;
pub mod csv_write;
pub mod dump;
pub mod encoding;
pub mod export;
pub mod import;
mod import_batch;
mod import_plan;
pub mod json_write;
pub mod mapping;
pub mod progress;
pub mod render;
pub mod report;
pub mod source;
pub mod sql_write;
pub mod target;
pub mod temporal;
#[cfg(test)]
mod test_support;
pub mod writer;
pub mod xlsx_read;
pub mod xlsx_write;

use serde::{Deserialize, Serialize};

/// 二进制值写成文本时的写法。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinaryEncoding {
    /// `0x` 开头的十六进制,和表格里输入二进制的写法一样。
    #[default]
    Hex,
    Base64,
}

/// 文本文件的编码。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextEncoding {
    #[default]
    Utf8,
    /// 带 BOM 的 UTF-8:Windows 上的 Excel 靠它认出 UTF-8。
    Utf8Bom,
    Gbk,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvOptions {
    pub delimiter: char,
    pub quote: char,
    /// 第一行写列名。
    pub header: bool,
    pub encoding: TextEncoding,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            quote: '"',
            header: true,
            encoding: TextEncoding::Utf8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportFormat {
    Csv(CsvOptions),
    Xlsx,
    /// `lines` 为真时每行一个 JSON 对象(JSON Lines),否则是一个数组。
    Json {
        lines: bool,
    },
    /// `table` 是 INSERT INTO 后面的表名;每 `rows_per_statement` 行合成一条 INSERT。
    #[serde(rename_all = "camelCase")]
    Sql {
        table: String,
        rows_per_statement: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub format: ExportFormat,
    #[serde(default)]
    pub binary: BinaryEncoding,
}

/// 导出的一列。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportColumn {
    pub name: String,
    /// 给人看的类型名;查询结果里认不出的类型为空。
    pub data_type: String,
    /// 生成列:SQL INSERT 里跳过(写不进去)。
    pub is_generated: bool,
}

impl ExportColumn {
    pub fn new(name: &str, data_type: &str) -> Self {
        Self {
            name: name.to_owned(),
            data_type: data_type.to_owned(),
            is_generated: false,
        }
    }
}

/// 一行里某一列的值没法写出来。`row` 从 1 开始数,不含表头。
pub(crate) fn value_error(row: u64, column: &str, reason: &str) -> crate::CoreError {
    crate::CoreError::InvalidData(format!("第 {row} 行「{column}」{reason}"))
}

/// 列名去重:JSON 的键、INSERT 的列名不能重复。查询结果里的同名列依次改成「名 (2)」「名 (3)」。
pub fn unique_names(columns: &[ExportColumn]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(columns.len());
    for column in columns {
        let mut name = column.name.clone();
        let mut n = 1;
        while seen.contains(&name) {
            n += 1;
            name = format!("{} ({n})", column.name);
        }
        seen.push(name);
    }
    seen
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn options_use_kind_tags_and_camel_case() {
        let options: ExportOptions = serde_json::from_value(json!({
            "format": { "kind": "csv", "delimiter": ";", "quote": "\"", "header": false, "encoding": "gbk" },
            "binary": "base64"
        }))
        .unwrap();
        assert_eq!(
            options.format,
            ExportFormat::Csv(CsvOptions {
                delimiter: ';',
                quote: '"',
                header: false,
                encoding: TextEncoding::Gbk,
            })
        );
        assert_eq!(options.binary, BinaryEncoding::Base64);

        let sql: ExportFormat = serde_json::from_value(
            json!({ "kind": "sql", "table": "users", "rowsPerStatement": 100 }),
        )
        .unwrap();
        assert_eq!(
            sql,
            ExportFormat::Sql {
                table: "users".into(),
                rows_per_statement: 100
            }
        );
        let defaulted: ExportOptions =
            serde_json::from_value(json!({ "format": { "kind": "xlsx" } })).unwrap();
        assert_eq!(defaulted.binary, BinaryEncoding::Hex);
        assert_eq!(
            serde_json::to_value(TextEncoding::Utf8Bom).unwrap(),
            json!("utf8Bom")
        );
    }

    #[test]
    fn duplicate_column_names_get_numbered() {
        let columns = [
            ExportColumn::new("id", ""),
            ExportColumn::new("name", ""),
            ExportColumn::new("id", ""),
            ExportColumn::new("id", ""),
            ExportColumn::new("id (2)", ""),
        ];
        assert_eq!(
            unique_names(&columns),
            ["id", "name", "id (2)", "id (3)", "id (2) (2)"]
        );
    }
}
