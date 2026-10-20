//! 三种方言共用的 DDL 片段。

use super::{FkAction, ForeignKeyDef, TableDesign};
use crate::sql::{Dialect, quote_ident};

/// 按字段 id 找名字;校验保证 id 都存在。
pub(super) fn name_of<'a>(design: &'a TableDesign, id: &str) -> &'a str {
    design
        .columns
        .iter()
        .find(|c| c.id == id)
        .map_or("", |c| c.name.as_str())
}

/// 带引号、逗号分隔的字段名列表。
pub(super) fn column_list(dialect: Dialect, design: &TableDesign, ids: &[String]) -> String {
    names(dialect, ids.iter().map(|id| name_of(design, id)))
}

pub(super) fn names<'a>(dialect: Dialect, names: impl IntoIterator<Item = &'a str>) -> String {
    names
        .into_iter()
        .map(|name| quote_ident(dialect, name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// ON DELETE / ON UPDATE;NO ACTION 是默认值,不写。
pub(super) fn actions(fk: &ForeignKeyDef) -> String {
    let mut sql = String::new();
    if fk.on_delete != FkAction::NoAction {
        sql.push_str(&format!(" ON DELETE {}", fk.on_delete.sql()));
    }
    if fk.on_update != FkAction::NoAction {
        sql.push_str(&format!(" ON UPDATE {}", fk.on_update.sql()));
    }
    sql
}

/// 外键的 `CONSTRAINT 名字 `前缀;没有名字时交给数据库。
pub(super) fn constraint_prefix(dialect: Dialect, name: &str) -> String {
    if name.trim().is_empty() {
        String::new()
    } else {
        format!("CONSTRAINT {} ", quote_ident(dialect, name))
    }
}

pub(super) fn non_empty(text: &Option<String>) -> Option<&str> {
    text.as_deref().filter(|t| !t.is_empty())
}

/// 多行定义体:每项一行、缩进两格。
pub(super) fn body(lines: &[String]) -> String {
    format!("(\n  {}\n)", lines.join(",\n  "))
}
