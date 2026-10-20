//! MySQL / MariaDB 的 DDL。
//!
//! DDL 会隐式提交,无法放进事务,所以尽量合成一条 ALTER TABLE:一条语句要么全生效、要么全不生效。
//! 只有删外键单独成一条放在最前面——同一条语句里先删后加同名外键、或删外键又删它用的索引,
//! 各版本的行为不一致。

use super::diff::{Diff, TargetColumn, ref_schema};
use super::render::{actions, body, constraint_prefix, name_of, non_empty};
use super::{
    ColumnDef, DdlContext, DdlPlan, DdlStatement, ExecMode, ForeignKeyDef, IndexDef, TableDesign,
    string_literal,
};
use crate::error::CoreResult;
use crate::sql::{Dialect, quote_ident, table_name};

const D: Dialect = Dialect::Mysql;
const INDENT: &str = "\n  ";

fn q(name: &str) -> String {
    quote_ident(D, name)
}

pub(super) fn plan(
    context: &DdlContext,
    original: Option<&TableDesign>,
    target: &TableDesign,
) -> CoreResult<DdlPlan> {
    let statements = match original {
        None => vec![create_table(context, target)],
        Some(original) => alter_table(context, &Diff::new(D, original, target)),
    };
    Ok(DdlPlan {
        mode: ExecMode::Autocommit,
        statements: statements.into_iter().map(DdlStatement::execute).collect(),
    })
}

fn create_table(context: &DdlContext, target: &TableDesign) -> String {
    let mut lines: Vec<String> = target
        .columns
        .iter()
        .map(|c| column_definition(context, c))
        .collect();
    if !target.primary_key.is_empty() {
        lines.push(format!(
            "PRIMARY KEY ({})",
            column_list(target, &target.primary_key)
        ));
    }
    lines.extend(target.indexes.iter().map(|i| index_definition(target, i)));
    lines.extend(
        target
            .foreign_keys
            .iter()
            .map(|f| foreign_key_definition(target, f)),
    );

    let options = &target.options;
    let mut suffix = String::new();
    if let Some(engine) = &options.engine {
        suffix.push_str(&format!(" ENGINE = {engine}"));
    }
    if let Some(charset) = &options.charset {
        suffix.push_str(&format!(" DEFAULT CHARSET = {charset}"));
    }
    if let Some(collation) = &options.collation {
        suffix.push_str(&format!(" COLLATE = {collation}"));
    }
    if let Some(comment) = non_empty(&options.comment) {
        suffix.push_str(&format!(" COMMENT = {}", literal(context, comment)));
    }
    format!(
        "CREATE TABLE {} {}{suffix}",
        table_name(D, &target.table),
        body(&lines)
    )
}

fn alter_table(context: &DdlContext, diff: &Diff) -> Vec<String> {
    let table = table_name(D, &diff.original.table);
    let alter = |clauses: Vec<String>| {
        format!(
            "ALTER TABLE {table}{INDENT}{}",
            clauses.join(&format!(",{INDENT}"))
        )
    };
    let mut statements = Vec::new();

    let fk = &diff.foreign_keys;
    let dropped_fks: Vec<String> = fk
        .dropped
        .iter()
        .copied()
        .chain(fk.renamed.iter().map(|(old, _)| *old))
        .map(|f| format!("DROP FOREIGN KEY {}", q(&f.name)))
        .collect();
    if !dropped_fks.is_empty() {
        statements.push(alter(dropped_fks));
    }

    let mut clauses = Vec::new();
    if diff.primary_key_changed && !diff.original.primary_key.is_empty() {
        clauses.push("DROP PRIMARY KEY".to_owned());
    }
    let indexes = &diff.indexes;
    for index in indexes
        .dropped
        .iter()
        .copied()
        .chain(indexes.renamed.iter().map(|(old, _)| *old))
    {
        clauses.push(format!("DROP INDEX {}", q(&index.name)));
    }
    for name in stale_support_indexes(diff) {
        clauses.push(format!("DROP INDEX {}", q(name)));
    }
    for column in &diff.dropped_columns {
        clauses.push(format!("DROP COLUMN {}", q(&column.name)));
    }
    clauses.extend(column_clauses(context, diff));
    if diff.primary_key_changed && !diff.target.primary_key.is_empty() {
        let columns = column_list(diff.target, &diff.target.primary_key);
        clauses.push(format!("ADD PRIMARY KEY ({columns})"));
    }
    // 按新结构里的顺序加:改名的在 MySQL 里也是先删再建。
    let readded = |id: &String, added: &[&IndexDef], renamed: &[(&IndexDef, &IndexDef)]| {
        added.iter().any(|i| &i.id == id) || renamed.iter().any(|(_, new)| &new.id == id)
    };
    for index in &diff.target.indexes {
        if readded(&index.id, &indexes.added, &indexes.renamed) {
            clauses.push(format!("ADD {}", index_definition(diff.target, index)));
        }
    }
    for f in &diff.target.foreign_keys {
        let readded =
            fk.added.iter().any(|a| a.id == f.id) || fk.renamed.iter().any(|(_, n)| n.id == f.id);
        if readded {
            clauses.push(format!("ADD {}", foreign_key_definition(diff.target, f)));
        }
    }
    clauses.extend(option_clauses(context, diff));
    if diff.table_renamed() {
        clauses.push(format!("RENAME TO {}", table_name(D, &diff.target.table)));
    }
    if !clauses.is_empty() {
        statements.push(alter(clauses));
    }
    statements
}

/// 外键改了字段、但名字没变:MySQL 当初自动建的支撑索引也叫这个名字、还在旧字段上,
/// 重新加外键时要再建一个同名索引,会报 Duplicate key name。这种自动索引先删掉,
/// MySQL 会按新字段重新建。用户自己建的同名索引不动。
fn stale_support_indexes<'a>(diff: &Diff<'a>) -> Vec<&'a str> {
    let (original, target) = (diff.original, diff.target);
    let fk = &diff.foreign_keys;
    let dropped = |name: &str| {
        let indexes = &diff.indexes;
        indexes
            .dropped
            .iter()
            .any(|i| i.name.eq_ignore_ascii_case(name))
            || indexes
                .renamed
                .iter()
                .any(|(old, _)| old.name.eq_ignore_ascii_case(name))
    };
    let covers = |columns: &[String], keys: &[String]| {
        keys.len() <= columns.len() && columns[..keys.len()] == *keys
    };
    let readded = fk
        .added
        .iter()
        .copied()
        .chain(fk.renamed.iter().map(|(_, new)| *new));
    let mut names = Vec::new();
    for added in readded.filter(|f| !f.name.trim().is_empty()) {
        let Some(old) = original
            .foreign_keys
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(&added.name))
        else {
            continue;
        };
        let Some(index) = original
            .indexes
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(&added.name) && !dropped(&i.name))
        else {
            continue;
        };
        let index_columns: Vec<String> = index.columns.iter().map(|c| c.column.clone()).collect();
        let covered = covers(&target.primary_key, &added.columns)
            || target.indexes.iter().any(|i| {
                let columns: Vec<String> = i.columns.iter().map(|c| c.column.clone()).collect();
                covers(&columns, &added.columns)
            });
        if index_columns == old.columns && !covered {
            names.push(index.name.as_str());
        }
    }
    names
}

/// 字段的改动按新顺序写:新字段、挪了位置的字段带 FIRST / AFTER,
/// 前一个字段要么没动、要么已经先就位,AFTER 引用的都是新名字。
fn column_clauses(context: &DdlContext, diff: &Diff) -> Vec<String> {
    let stable = diff.stable_columns();
    let columns = &diff.columns;
    let mut clauses = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        let position = match index {
            0 => " FIRST".to_owned(),
            _ => format!(" AFTER {}", q(&columns[index - 1].after.name)),
        };
        let definition = column_definition(context, column.after);
        match column.before {
            None => {
                let appended = columns[index..].iter().all(|c| c.before.is_none());
                let position = if appended { "" } else { &position };
                clauses.push(format!("ADD COLUMN {definition}{position}"));
            }
            Some(before) => {
                let moved = !stable[index];
                if !moved && !column.changed() && !column.renamed() {
                    continue;
                }
                let position = if moved { position.as_str() } else { "" };
                clauses.push(modify_clause(column, before, &definition, position));
            }
        }
    }
    clauses
}

fn modify_clause(
    column: &TargetColumn,
    before: &ColumnDef,
    definition: &str,
    position: &str,
) -> String {
    if column.renamed() {
        format!("CHANGE COLUMN {} {definition}{position}", q(&before.name))
    } else {
        format!("MODIFY COLUMN {definition}{position}")
    }
}

fn option_clauses(context: &DdlContext, diff: &Diff) -> Vec<String> {
    let (before, after) = (&diff.original.options, &diff.target.options);
    let mut clauses = Vec::new();
    if before.engine != after.engine
        && let Some(engine) = &after.engine
    {
        clauses.push(format!("ENGINE = {engine}"));
    }
    if before.charset != after.charset || before.collation != after.collation {
        match (&after.charset, &after.collation) {
            (Some(charset), Some(collation)) => clauses.push(format!(
                "DEFAULT CHARACTER SET = {charset} COLLATE = {collation}"
            )),
            (Some(charset), None) => clauses.push(format!("DEFAULT CHARACTER SET = {charset}")),
            (None, Some(collation)) => clauses.push(format!("DEFAULT COLLATE = {collation}")),
            (None, None) => {}
        }
    }
    if non_empty(&before.comment) != non_empty(&after.comment) {
        let comment = non_empty(&after.comment).unwrap_or_default();
        clauses.push(format!("COMMENT = {}", literal(context, comment)));
    }
    clauses
}

fn column_definition(context: &DdlContext, column: &ColumnDef) -> String {
    let data_type = column.data_type.trim();
    let mut sql = format!("{} {data_type}", q(&column.name));
    if let Some(collation) = &column.collation
        && is_string_type(data_type)
    {
        sql.push_str(&format!(" COLLATE {collation}"));
    }
    if let Some(generated) = &column.generated {
        let kind = if generated.stored {
            "STORED"
        } else {
            "VIRTUAL"
        };
        sql.push_str(&format!(
            " GENERATED ALWAYS AS ({}) {kind}",
            generated.expression.trim()
        ));
    }
    // 显式写 NULL:MySQL 5.7 默认配置下不写的 TIMESTAMP 列会变成 NOT NULL。
    sql.push_str(if column.nullable {
        " NULL"
    } else {
        " NOT NULL"
    });
    if let Some(default) = &column.default {
        sql.push_str(&format!(" DEFAULT {}", default.trim()));
    }
    if let Some(on_update) = &column.on_update {
        sql.push_str(&format!(" ON UPDATE {}", on_update.trim()));
    }
    if column.auto_increment {
        sql.push_str(" AUTO_INCREMENT");
    }
    if let Some(comment) = non_empty(&column.comment) {
        sql.push_str(&format!(" COMMENT {}", literal(context, comment)));
    }
    sql
}

/// 字符串类型才能带 COLLATE;用户把类型改成数字时,读出来的排序规则就不再写。
fn is_string_type(data_type: &str) -> bool {
    let head: String = data_type
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        head.as_str(),
        "char"
            | "varchar"
            | "tinytext"
            | "text"
            | "mediumtext"
            | "longtext"
            | "enum"
            | "set"
            | "character"
            | "national"
            | "nchar"
            | "nvarchar"
    )
}

fn index_definition(target: &TableDesign, index: &IndexDef) -> String {
    let method = index.method.as_deref().map(str::to_ascii_uppercase);
    let (kind, using) = match method.as_deref() {
        Some(prefix @ ("FULLTEXT" | "SPATIAL")) => (format!("{prefix} INDEX"), String::new()),
        Some(other) => (plain_kind(index), format!(" USING {other}")),
        None => (plain_kind(index), String::new()),
    };
    let columns: Vec<String> = index
        .columns
        .iter()
        .map(|c| {
            let mut part = q(name_of(target, &c.column));
            if let Some(length) = c.length {
                part.push_str(&format!("({length})"));
            }
            if c.descending {
                part.push_str(" DESC");
            }
            part
        })
        .collect();
    format!("{kind} {} ({}){using}", q(&index.name), columns.join(", "))
}

fn plain_kind(index: &IndexDef) -> String {
    if index.unique {
        "UNIQUE INDEX"
    } else {
        "INDEX"
    }
    .to_owned()
}

fn foreign_key_definition(target: &TableDesign, fk: &ForeignKeyDef) -> String {
    let mut sql = constraint_prefix(D, &fk.name);
    let referenced: Vec<String> = fk.ref_columns.iter().map(|c| q(c)).collect();
    sql.push_str(&format!(
        "FOREIGN KEY ({}) REFERENCES {}.{} ({})",
        column_list(target, &fk.columns),
        q(ref_schema(fk, target)),
        q(&fk.ref_table),
        referenced.join(", ")
    ));
    sql.push_str(&actions(fk));
    sql
}

fn column_list(target: &TableDesign, ids: &[String]) -> String {
    super::render::column_list(D, target, ids)
}

fn literal(context: &DdlContext, text: &str) -> String {
    string_literal(D, context.backslash_escapes, text)
}
