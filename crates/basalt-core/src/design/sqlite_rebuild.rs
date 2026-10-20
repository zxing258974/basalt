//! SQLite 重建表:按官方流程建新表 → 复制数据 → 删旧表 → 改名 → 重建索引和触发器,
//! 提交前做外键检查(执行见驱动)。重建前先查有没有触发器、视图等按原文保留的东西
//! 引用了要改名、删除的字段或表。

use super::diff::{Diff, TargetColumn};
use super::render::names;
use super::sqlite::{D, create_index, q, table_definition};
use super::sqlite_parse::identifiers;
use super::{
    DdlContext, DdlPlan, DdlStatement, ExecMode, IndexDef, StatementKind, TableDesign,
    string_literal,
};
use crate::error::{CoreError, CoreResult};
use crate::sql::table_name;

/// 重建表时,触发器、视图、检查约束这些按原文重建或原样留着的东西跟不上改名和删字段。
/// 发现它们引用了要改名、删除的字段或表,就拒绝,让用户分两次做。
pub(super) fn check_dependents(context: &DdlContext, diff: &Diff) -> CoreResult<()> {
    let original = diff.original;
    let table = original.table.table.to_lowercase();
    let renamed: Vec<&str> = diff
        .columns
        .iter()
        .filter(|c| c.renamed())
        .filter_map(|c| c.before.map(|b| b.name.as_str()))
        .collect();
    let dropped: Vec<&str> = diff
        .dropped_columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    if renamed.is_empty() && dropped.is_empty() && !diff.table_renamed() {
        return Ok(());
    }
    for (label, sql, own) in dependents(context, diff) {
        let ids = identifiers(&sql);
        let mentions = |name: &str| ids.contains(&name.to_lowercase());
        if !own && !mentions(&table) {
            continue;
        }
        if diff.table_renamed() && mentions(&table) {
            return Err(split(
                format!("{label}引用了表名「{}」", original.table.table),
                "表名",
            ));
        }
        if let Some(column) = renamed.iter().find(|c| mentions(c)) {
            return Err(split(
                format!("{label}引用了要改名的字段「{column}」"),
                "字段名",
            ));
        }
        if let Some(column) = dropped.iter().find(|c| mentions(c)) {
            return Err(CoreError::InvalidEdit(format!(
                "SQLite 要重建这张表才能完成这次修改，但{label}引用了要删除的字段「{column}」。请先修改或删除{label}，再删这个字段。"
            )));
        }
    }
    Ok(())
}

fn split(reason: String, what: &str) -> CoreError {
    CoreError::InvalidEdit(format!(
        "SQLite 要重建这张表才能完成这次修改，但{reason}，重建时没法跟着改。请分两次保存：先只改{what}并保存（SQLite 会自动更新引用），再做其余修改。"
    ))
}

/// (说明, SQL 原文, 是否属于这张表)。
fn dependents(context: &DdlContext, diff: &Diff) -> Vec<(String, String, bool)> {
    let original = diff.original;
    let mut out: Vec<(String, String, bool)> = Vec::new();
    for trigger in &original.triggers {
        out.push((
            format!("触发器「{}」", trigger.name),
            trigger.sql.clone(),
            true,
        ));
    }
    for (_, index) in kept_raw_indexes(diff) {
        let definition = index.definition.clone().unwrap_or_default();
        out.push((format!("索引「{}」", index.name), definition, true));
    }
    for check in &original.checks {
        out.push(("检查约束".into(), check.expression.clone(), true));
    }
    for column in diff.columns.iter().filter_map(|c| c.before) {
        if let Some(generated) = &column.generated {
            out.push((
                format!("生成列「{}」", column.name),
                generated.expression.clone(),
                true,
            ));
        }
    }
    for object in &context.schema_objects {
        let kind = match object.kind.as_str() {
            "view" => "视图",
            "trigger" => "触发器",
            "index" => "索引",
            _ => "表",
        };
        out.push((
            format!("{kind}「{}」", object.name),
            object.sql.clone(),
            false,
        ));
    }
    out
}

/// 保留下来的表达式索引、部分索引:(原定义, 新定义)。
fn kept_raw_indexes<'a>(diff: &Diff<'a>) -> Vec<(&'a IndexDef, &'a IndexDef)> {
    diff.target
        .indexes
        .iter()
        .filter(|i| i.definition.is_some())
        .filter_map(|after| {
            let before = diff.original.indexes.iter().find(|b| b.id == after.id)?;
            Some((before, after))
        })
        .collect()
}

pub(super) fn rebuild(context: &DdlContext, diff: &Diff) -> DdlPlan {
    let (original, target) = (diff.original, diff.target);
    let schema = &original.table.database;
    let old = table_name(D, &original.table);
    let temporary = format!("_basalt_new_{}", target.table.table);
    let new = format!("{}.{}", q(schema), q(&temporary));
    let constraint = |index: &IndexDef| {
        original
            .indexes
            .iter()
            .any(|b| b.id == index.id && b.is_constraint)
    };
    // 表改了名:引用自己的外键也要指向新名字,否则重建后引用的是一张不存在的表。
    let mut renamed = target.clone();
    for fk in &mut renamed.foreign_keys {
        if fk.ref_table.eq_ignore_ascii_case(&original.table.table) {
            fk.ref_table.clone_from(&target.table.table);
        }
    }
    let mut sql = vec![
        "PRAGMA legacy_alter_table = ON".to_owned(),
        table_definition(
            &new,
            &renamed,
            &constraint,
            &original.checks,
            &original.options,
        ),
    ];
    let copied: Vec<&TargetColumn> = diff
        .columns
        .iter()
        .filter(|c| c.before.is_some_and(|b| b.generated.is_none()) && c.after.generated.is_none())
        .collect();
    if !copied.is_empty() {
        let into = names(D, copied.iter().map(|c| c.after.name.as_str()));
        let from = names(
            D,
            copied
                .iter()
                .filter_map(|c| c.before.map(|b| b.name.as_str())),
        );
        sql.push(format!(
            "INSERT INTO {new} ({into}) SELECT {from} FROM {old}"
        ));
    }
    let autoincrement = |design: &TableDesign| design.columns.iter().any(|c| c.auto_increment);
    if autoincrement(original) && autoincrement(target) {
        let sequence = format!("{}.sqlite_sequence", q(schema));
        let temporary_name = string_literal(D, false, &temporary);
        let old_name = string_literal(D, false, &original.table.table);
        sql.push(format!(
            "DELETE FROM {sequence} WHERE name = {temporary_name}"
        ));
        sql.push(format!(
            "INSERT INTO {sequence} (name, seq) SELECT {temporary_name}, seq FROM {sequence} WHERE name = {old_name}"
        ));
    }
    sql.push(format!("DROP TABLE {old}"));
    sql.push(format!(
        "ALTER TABLE {new} RENAME TO {}",
        q(&target.table.table)
    ));
    sql.push("PRAGMA legacy_alter_table = OFF".to_owned());
    for index in target.indexes.iter().filter(|i| !constraint(i)) {
        match &index.definition {
            Some(definition) => sql.push(definition.clone()),
            None => sql.push(create_index(target, index)),
        }
    }
    sql.extend(original.triggers.iter().map(|t| t.sql.clone()));

    let mut statements: Vec<DdlStatement> = sql.into_iter().map(DdlStatement::execute).collect();
    let checked = std::iter::once(&target.table.table).chain(
        context
            .referencing_tables
            .iter()
            .filter(|t| !t.eq_ignore_ascii_case(&original.table.table)),
    );
    for table in checked {
        statements.push(DdlStatement {
            sql: format!("PRAGMA {}.foreign_key_check({})", q(schema), q(table)),
            kind: StatementKind::ForeignKeyCheck,
        });
    }
    DdlPlan {
        mode: ExecMode::Rebuild,
        statements,
    }
}
