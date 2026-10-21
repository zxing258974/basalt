//! 信息面板:一张表(或视图)的属性和建表语句。
//!
//! 属性按各家能给的列:MySQL 是 information_schema.TABLES 那一套(和 Navicat 的一样),
//! PostgreSQL 是估算行数、各部分大小、所有者、最后清理与分析的时间,SQLite 是行数和表选项。
//! 建表语句 MySQL 用 SHOW CREATE TABLE 的原文、SQLite 用 sqlite_master 里存的原文;
//! PostgreSQL 没有现成的,按读出来的表结构生成(视图用 pg_get_viewdef)。

use serde::Serialize;

use crate::design::postgres_dump_statements;
use crate::driver::table_not_found;
use crate::error::{CoreError, CoreResult};
use crate::session::Session;
use crate::table::TableRef;

/// 一个属性的值。数字按类型交给前端排版(字节换算成 KB / MB、千分位)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum PropertyValue {
    Text(String),
    Count(u64),
    Bytes(u64),
    /// 数据库给的时间原文,如 `2026-08-10 09:47:02`。
    DateTime(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableProperty {
    pub label: String,
    /// 数据库没给(如没开统计、从没分析过)时为 `None`,前端显示「—」。
    pub value: Option<PropertyValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetails {
    pub properties: Vec<TableProperty>,
    pub ddl: String,
    /// 建表语句是按表结构生成的(PostgreSQL),不是数据库给的原文。
    pub ddl_generated: bool,
}

/// 转储用的建表语句,按执行时机分开:建表在插数据之前;外键、触发器在插完数据之后,
/// 免得自引用的行顺序不对、还原时每插一行就触发一次。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreateScript {
    /// 建表、建索引、注释。
    pub table: Vec<String>,
    /// PostgreSQL 单独给出的外键;MySQL、SQLite 的外键写在建表语句里。
    pub foreign_keys: Vec<String>,
    /// 每条一个 CREATE TRIGGER,末尾不带分号。
    pub triggers: Vec<String>,
}

/// 收集属性的小帮手:按顺序记下标签和值。
#[derive(Default)]
pub(crate) struct Properties(Vec<TableProperty>);

impl Properties {
    fn push(&mut self, label: &str, value: Option<PropertyValue>) {
        self.0.push(TableProperty {
            label: label.to_owned(),
            value,
        });
    }

    pub(crate) fn text(&mut self, label: &str, value: Option<String>) {
        let value = value.filter(|v| !v.trim().is_empty());
        self.push(label, value.map(PropertyValue::Text));
    }

    /// 数字以文本给出(各家整数类型、无符号与否都不一样,统一 CAST 成文本再解析)。
    pub(crate) fn count(&mut self, label: &str, value: Option<String>) {
        self.push(label, parse_number(value).map(PropertyValue::Count));
    }

    pub(crate) fn bytes(&mut self, label: &str, value: Option<String>) {
        self.push(label, parse_number(value).map(PropertyValue::Bytes));
    }

    pub(crate) fn datetime(&mut self, label: &str, value: Option<String>) {
        let value = value.filter(|v| !v.trim().is_empty());
        self.push(
            label,
            value.map(|v| PropertyValue::DateTime(trim_fraction(&v))),
        );
    }

    pub(crate) fn into_vec(self) -> Vec<TableProperty> {
        self.0
    }
}

/// 负数(如 PostgreSQL 没分析过时 reltuples 是 -1)和解析不了的都当作没有。
fn parse_number(value: Option<String>) -> Option<u64> {
    let text = value?;
    let text = text.trim();
    text.parse::<u64>().ok().or_else(|| {
        text.parse::<f64>()
            .ok()
            .filter(|f| *f >= 0.0)
            .map(|f| f.round() as u64)
    })
}

/// `2026-10-21 08:05:09.123456+08` → `2026-10-21 08:05:09+08`:信息面板不需要微秒。
fn trim_fraction(text: &str) -> String {
    match text.find('.') {
        Some(dot) => {
            let tail = &text[dot + 1..];
            let digits = tail.chars().take_while(char::is_ascii_digit).count();
            format!("{}{}", &text[..dot], &tail[digits..])
        }
        None => text.to_owned(),
    }
}

/// 多条语句拼成一段:每条末尾加分号,中间空一行。
pub(crate) fn join_statements(statements: impl IntoIterator<Item = String>) -> String {
    statements
        .into_iter()
        .map(|s| {
            let s = s.trim_end().trim_end_matches(';').to_owned();
            format!("{s};")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl Session {
    pub async fn table_details(&self, table: &TableRef) -> CoreResult<TableDetails> {
        if table.table.is_empty() {
            return Err(table_not_found(""));
        }
        match self {
            Self::Mysql(s) => s.table_details(table).await,
            Self::Postgres(s) => {
                if table.schema.is_none() {
                    return Err(CoreError::Unsupported("PostgreSQL 需要指定 schema".into()));
                }
                s.table_details(self, table).await
            }
            Self::Sqlite(s) => s.table_details(table).await,
        }
    }

    /// 一张表的建表语句,来源和信息面板一样:MySQL 是 SHOW CREATE TABLE、SQLite 是 `sqlite_master` 的原文
    /// (连同索引),PostgreSQL 按表结构生成(建表、索引、注释,外键另给);再加上表上的触发器。
    pub async fn create_script(&self, table: &TableRef) -> CoreResult<CreateScript> {
        self.check_table(table)?;
        Ok(match self {
            Self::Mysql(s) => CreateScript {
                table: vec![s.show_create(table).await?],
                foreign_keys: Vec::new(),
                triggers: s.trigger_statements(table).await?,
            },
            Self::Postgres(s) => {
                let (create, foreign_keys) =
                    postgres_dump_statements(&self.read_design(table).await?);
                CreateScript {
                    table: create,
                    foreign_keys,
                    triggers: s.trigger_statements(table).await?,
                }
            }
            Self::Sqlite(s) => {
                let (triggers, rest): (Vec<_>, Vec<_>) = s
                    .own_objects(table)
                    .await?
                    .into_iter()
                    .partition(|(kind, _)| kind == "trigger");
                CreateScript {
                    table: rest.into_iter().map(|(_, sql)| sql).collect(),
                    foreign_keys: Vec::new(),
                    triggers: triggers.into_iter().map(|(_, sql)| sql).collect(),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_times_are_cleaned_up() {
        assert_eq!(parse_number(Some("16384".into())), Some(16384));
        assert_eq!(parse_number(Some("1.5e3".into())), Some(1500));
        assert_eq!(parse_number(Some("-1".into())), None);
        assert_eq!(parse_number(Some("abc".into())), None);
        assert_eq!(parse_number(None), None);
        assert_eq!(
            trim_fraction("2026-10-21 08:05:09.123456+08"),
            "2026-10-21 08:05:09+08"
        );
        assert_eq!(trim_fraction("2026-08-10 09:47:02"), "2026-08-10 09:47:02");
    }

    #[test]
    fn empty_text_shows_as_missing() {
        let mut properties = Properties::default();
        properties.text("注释", Some("  ".into()));
        properties.text("引擎", Some("InnoDB".into()));
        properties.bytes("数据长度", Some("16384".into()));
        assert_eq!(
            serde_json::to_value(properties.into_vec()).unwrap(),
            serde_json::json!([
                { "label": "注释", "value": null },
                { "label": "引擎", "value": { "kind": "text", "value": "InnoDB" } },
                { "label": "数据长度", "value": { "kind": "bytes", "value": 16384 } },
            ])
        );
    }

    #[test]
    fn statements_are_joined_with_semicolons() {
        assert_eq!(
            join_statements([
                "CREATE TABLE t (a int)".to_owned(),
                "CREATE INDEX i ON t (a);\n".to_owned()
            ]),
            "CREATE TABLE t (a int);\n\nCREATE INDEX i ON t (a);"
        );
    }
}
