//! 生成 DDL 前检查用户改出来的结构。原文片段(类型、默认值、表达式)会原样拼进 DDL,
//! 只做词法检查:不许有语句外的分号,括号、引号要配对。

use std::collections::HashSet;

use super::{
    COLUMN_ID_PREFIX, ColumnDef, FOREIGN_KEY_ID_PREFIX, ForeignKeyDef, INDEX_ID_PREFIX, IndexDef,
    TableDesign,
};
use crate::error::{CoreError, CoreResult};
use crate::sql::Dialect;
use crate::sql::filter::check_filter;

fn invalid<T>(message: String) -> CoreResult<T> {
    Err(CoreError::InvalidEdit(message))
}

pub(super) fn validate(
    dialect: Dialect,
    original: Option<&TableDesign>,
    target: &TableDesign,
) -> CoreResult<()> {
    if target.table.table.trim().is_empty() {
        return invalid("请填写表名。".into());
    }
    if let Some(original) = original
        && (original.table.database != target.table.database
            || original.table.schema != target.table.schema)
    {
        return invalid("不能把表挪到别的库或 schema。".into());
    }
    columns(dialect, &target.columns)?;
    primary_key(target)?;
    indexes(dialect, original, target)?;
    foreign_keys(dialect, target)?;
    options(target)?;
    if dialect == Dialect::Sqlite {
        sqlite_autoincrement(target)?;
    }
    match original {
        Some(original) => stale_ids(original, target)?,
        // 新建 SQLite 表时检查约束原样写进 CREATE TABLE。
        None => {
            for check in &target.checks {
                fragment(dialect, &check.expression, "检查约束")?;
            }
        }
    }
    Ok(())
}

/// 同名判断:PostgreSQL 区分大小写,另外两种不区分。
pub(super) fn name_key(dialect: Dialect, name: &str) -> String {
    match dialect {
        Dialect::Postgres => name.to_owned(),
        Dialect::Mysql | Dialect::Sqlite => name.to_lowercase(),
    }
}

/// 原样拼进 DDL 的片段:不能为空白,语句外不能有分号,括号、引号要配对。
fn fragment(dialect: Dialect, text: &str, what: &str) -> CoreResult<()> {
    if text.trim().is_empty() {
        return invalid(format!("{what}是空的。"));
    }
    check_filter(dialect, text)
        .or_else(|_| invalid(format!("{what}有问题：不能有分号，括号和引号要配对。")))
}

/// 引擎、字符集、排序规则这类名字不加引号直接拼,只许字母、数字和下划线。
fn word(text: &str, what: &str) -> CoreResult<()> {
    let ok = !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        invalid(format!("{what}「{text}」只能由字母、数字和下划线组成。"))
    }
}

fn columns(dialect: Dialect, columns: &[ColumnDef]) -> CoreResult<()> {
    if columns.is_empty() {
        return invalid("表里至少要有一个字段。".into());
    }
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for (index, column) in columns.iter().enumerate() {
        let name = &column.name;
        if name.trim().is_empty() {
            return invalid(format!("第 {} 个字段没有填字段名。", index + 1));
        }
        if !names.insert(name_key(dialect, name)) {
            return invalid(format!("字段名「{name}」重复了。"));
        }
        if !ids.insert(column.id.as_str()) {
            return invalid(format!("字段「{name}」的 id 重复了，请重新打开设计器。"));
        }
        column_attributes(dialect, column)?;
    }
    if dialect == Dialect::Mysql && columns.iter().filter(|c| c.auto_increment).count() > 1 {
        return invalid("MySQL 一张表只能有一个自增字段。".into());
    }
    Ok(())
}

fn column_attributes(dialect: Dialect, column: &ColumnDef) -> CoreResult<()> {
    let name = &column.name;
    if column.data_type.trim().is_empty() {
        if dialect != Dialect::Sqlite {
            return invalid(format!("字段「{name}」没有填类型。"));
        }
    } else {
        fragment(dialect, &column.data_type, &format!("字段「{name}」的类型"))?;
    }
    if let Some(default) = &column.default {
        fragment(dialect, default, &format!("字段「{name}」的默认值"))?;
    }
    if let Some(on_update) = &column.on_update {
        fragment(dialect, on_update, &format!("字段「{name}」的 ON UPDATE"))?;
    }
    if let Some(collation) = &column.collation {
        let what = format!("字段「{name}」的排序规则");
        // PostgreSQL 的排序规则读出来就带 schema 和引号,如 pg_catalog."C"。
        match dialect {
            Dialect::Postgres => fragment(dialect, collation, &what)?,
            Dialect::Mysql | Dialect::Sqlite => word(collation, &what)?,
        }
    }
    if let Some(generated) = &column.generated {
        fragment(
            dialect,
            &generated.expression,
            &format!("字段「{name}」的生成表达式"),
        )?;
        if column.default.is_some() {
            return invalid(format!("字段「{name}」是生成列，不能设默认值。"));
        }
        if column.auto_increment {
            return invalid(format!("字段「{name}」是生成列，不能自增。"));
        }
    }
    if column.auto_increment && column.default.is_some() {
        return invalid(format!("字段「{name}」是自增字段，不能再设默认值。"));
    }
    Ok(())
}

fn column_name<'a>(target: &'a TableDesign, id: &str) -> Option<&'a str> {
    target
        .columns
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.name.as_str())
}

/// 字段 id 列表:都要存在,不能重复。`what` 用来拼提示语。
fn column_refs(target: &TableDesign, ids: &[String], what: &str) -> CoreResult<()> {
    let mut seen = HashSet::new();
    for id in ids {
        let Some(name) = column_name(target, id) else {
            return invalid(format!("{what}引用的字段不存在了。"));
        };
        if !seen.insert(id) {
            return invalid(format!("{what}里字段「{name}」重复了。"));
        }
    }
    Ok(())
}

fn primary_key(target: &TableDesign) -> CoreResult<()> {
    column_refs(target, &target.primary_key, "主键")
}

fn indexes(
    dialect: Dialect,
    original: Option<&TableDesign>,
    target: &TableDesign,
) -> CoreResult<()> {
    let mut names = HashSet::new();
    for index in &target.indexes {
        if index.definition.is_some() {
            raw_index_unchanged(original, index)?;
            continue;
        }
        let name = &index.name;
        if name.trim().is_empty() {
            return invalid("有索引没有填索引名。".into());
        }
        if !names.insert(name_key(dialect, name)) {
            return invalid(format!("索引名「{name}」重复了。"));
        }
        if index.columns.is_empty() {
            return invalid(format!("索引「{name}」至少选一个字段。"));
        }
        let ids: Vec<String> = index.columns.iter().map(|c| c.column.clone()).collect();
        column_refs(target, &ids, &format!("索引「{name}」"))?;
        if let Some(method) = &index.method {
            word(method, &format!("索引「{name}」的索引类型"))?;
        }
    }
    Ok(())
}

/// 表达式索引、部分索引只能原样保留或删除。
fn raw_index_unchanged(original: Option<&TableDesign>, index: &IndexDef) -> CoreResult<()> {
    let before = original.and_then(|o| o.indexes.iter().find(|i| i.id == index.id));
    match before {
        Some(before) if before.definition == index.definition => Ok(()),
        _ => invalid(format!(
            "不能新建或修改索引「{}」这种索引（表达式索引、部分索引等），只能删除。请在 SQL 编辑器里建。",
            index.name
        )),
    }
}

fn foreign_keys(dialect: Dialect, target: &TableDesign) -> CoreResult<()> {
    let mut names = HashSet::new();
    for (index, fk) in target.foreign_keys.iter().enumerate() {
        let label = foreign_key_label(fk, index);
        if !fk.name.trim().is_empty() && !names.insert(name_key(dialect, &fk.name)) {
            return invalid(format!("外键名「{}」重复了。", fk.name));
        }
        if fk.columns.is_empty() {
            return invalid(format!("{label}至少选一个字段。"));
        }
        column_refs(target, &fk.columns, &label)?;
        if fk.ref_table.trim().is_empty() {
            return invalid(format!("{label}没有选引用的表。"));
        }
        if fk.ref_columns.iter().any(|c| c.trim().is_empty()) {
            return invalid(format!("{label}有引用的字段是空的。"));
        }
        let may_omit = dialect != Dialect::Mysql && fk.ref_columns.is_empty();
        if !may_omit && fk.ref_columns.len() != fk.columns.len() {
            return invalid(format!("{label}的字段和引用的字段个数不一致。"));
        }
    }
    Ok(())
}

fn foreign_key_label(fk: &ForeignKeyDef, index: usize) -> String {
    if fk.name.trim().is_empty() {
        format!("第 {} 个外键", index + 1)
    } else {
        format!("外键「{}」", fk.name)
    }
}

fn options(target: &TableDesign) -> CoreResult<()> {
    let options = &target.options;
    if let Some(engine) = &options.engine {
        word(engine, "存储引擎")?;
    }
    if let Some(charset) = &options.charset {
        word(charset, "字符集")?;
    }
    if let Some(collation) = &options.collation {
        word(collation, "排序规则")?;
    }
    Ok(())
}

/// SQLite 的 AUTOINCREMENT 只能加在单独作主键的 INTEGER 字段上。
fn sqlite_autoincrement(target: &TableDesign) -> CoreResult<()> {
    for column in target.columns.iter().filter(|c| c.auto_increment) {
        let lone_key = target.primary_key.len() == 1 && target.primary_key[0] == column.id;
        if !lone_key || !column.data_type.trim().eq_ignore_ascii_case("INTEGER") {
            return invalid(format!(
                "SQLite 只有单独作主键、类型是 INTEGER 的字段才能自增（字段「{}」）。",
                column.name
            ));
        }
    }
    Ok(())
}

/// 带着「读出来的」id 却在库里找不到:打开设计器之后表结构被别人改了。
fn stale_ids(original: &TableDesign, target: &TableDesign) -> CoreResult<()> {
    let stale = |what: &str, name: &str| {
        invalid(format!(
            "{what}「{name}」在库里已经不存在了，可能有人改过这张表。请重新读取表结构后再修改。"
        ))
    };
    for column in &target.columns {
        let known = original.columns.iter().any(|c| c.id == column.id);
        if column.id.starts_with(COLUMN_ID_PREFIX) && !known {
            return stale("字段", &column.name);
        }
    }
    for index in &target.indexes {
        let known = original.indexes.iter().any(|i| i.id == index.id);
        if index.id.starts_with(INDEX_ID_PREFIX) && !known {
            return stale("索引", &index.name);
        }
    }
    for fk in &target.foreign_keys {
        let known = original.foreign_keys.iter().any(|f| f.id == fk.id);
        if fk.id.starts_with(FOREIGN_KEY_ID_PREFIX) && !known {
            return stale("外键", &fk.name);
        }
    }
    Ok(())
}
