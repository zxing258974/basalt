//! PostgreSQL 的 DDL。所有语句放在一个事务里执行,所以每个动作单独一条,读起来清楚。
//!
//! 顺序:删外键 → 删索引 → 删主键 → 删字段 → 字段改名 → 改字段 → 加字段 → 加主键 →
//! 索引改名、建索引 → 外键改名、加外键 → 注释 → 表改名。改名放在改字段之前,
//! 后面的语句一律用新字段名;表改名放在最后,前面的语句一律用旧表名。

use super::diff::{Diff, TargetColumn, ref_schema};
use super::render::{actions, body, column_list, constraint_prefix, name_of, non_empty};
use super::sqlite_parse::identifiers;
use super::{
    ColumnDef, DdlPlan, DdlStatement, ExecMode, ForeignKeyDef, IndexDef, TableDesign,
    string_literal,
};
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, qualified, quote_ident, table_name};

const D: Dialect = Dialect::Postgres;

fn q(name: &str) -> String {
    quote_ident(D, name)
}

pub(super) fn plan(original: Option<&TableDesign>, target: &TableDesign) -> CoreResult<DdlPlan> {
    let statements = match original {
        None => create_table(target),
        Some(original) => alter_table(&Diff::new(D, original, target))?,
    };
    Ok(DdlPlan {
        mode: ExecMode::Transaction,
        statements: statements.into_iter().map(DdlStatement::execute).collect(),
    })
}

fn schema(design: &TableDesign) -> &str {
    design.table.schema.as_deref().unwrap_or_default()
}

fn create_table(target: &TableDesign) -> Vec<String> {
    let mut lines: Vec<String> = target.columns.iter().map(column_definition).collect();
    if !target.primary_key.is_empty() {
        let name = target.primary_key_name.as_deref().unwrap_or_default();
        lines.push(primary_key(target, name));
    }
    lines.extend(
        target
            .foreign_keys
            .iter()
            .map(|f| foreign_key_definition(target, f)),
    );
    let table = table_name(D, &target.table);
    let mut statements = vec![format!("CREATE TABLE {table} {}", body(&lines))];
    statements.extend(target.indexes.iter().map(|i| create_index(target, i)));
    if let Some(comment) = non_empty(&target.options.comment) {
        statements.push(format!("COMMENT ON TABLE {table} IS {}", literal(comment)));
    }
    for column in &target.columns {
        if let Some(comment) = non_empty(&column.comment) {
            statements.push(column_comment(target, &column.name, Some(comment)));
        }
    }
    statements
}

fn alter_table(diff: &Diff) -> CoreResult<Vec<String>> {
    check_order(diff)?;
    check_raw_indexes(diff)?;
    let (original, target) = (diff.original, diff.target);
    let table = table_name(D, &original.table);
    let alter = |action: String| format!("ALTER TABLE {table} {action}");
    let mut out = Vec::new();

    for fk in &diff.foreign_keys.dropped {
        out.push(alter(format!("DROP CONSTRAINT {}", q(&fk.name))));
    }
    for index in &diff.indexes.dropped {
        out.push(if index.is_constraint {
            alter(format!("DROP CONSTRAINT {}", q(&index.name)))
        } else {
            format!(
                "DROP INDEX {}",
                qualified(D, &[schema(original), &index.name])
            )
        });
    }
    let key_name = original
        .primary_key_name
        .clone()
        .unwrap_or_else(|| format!("{}_pkey", original.table.table));
    if diff.primary_key_changed && !original.primary_key.is_empty() {
        out.push(alter(format!("DROP CONSTRAINT {}", q(&key_name))));
    }
    for column in &diff.dropped_columns {
        out.push(alter(format!("DROP COLUMN {}", q(&column.name))));
    }
    for (from, to) in diff.rename_steps() {
        out.push(alter(format!("RENAME COLUMN {} TO {}", q(&from), q(&to))));
    }
    for column in &diff.columns {
        out.extend(alter_column(column)?.into_iter().map(&alter));
    }
    for column in diff.columns.iter().filter(|c| c.before.is_none()) {
        out.push(alter(format!(
            "ADD COLUMN {}",
            column_definition(column.after)
        )));
    }
    if diff.primary_key_changed && !target.primary_key.is_empty() {
        let name = target.primary_key_name.as_deref().unwrap_or(&key_name);
        out.push(alter(format!("ADD {}", primary_key(target, name))));
    }
    for (old, new) in &diff.indexes.renamed {
        let old_name = qualified(D, &[schema(original), &old.name]);
        out.push(format!("ALTER INDEX {old_name} RENAME TO {}", q(&new.name)));
    }
    out.extend(
        diff.indexes
            .added
            .iter()
            .map(|i| create_index_on(&table, i, target)),
    );
    for (old, new) in &diff.foreign_keys.renamed {
        out.push(alter(format!(
            "RENAME CONSTRAINT {} TO {}",
            q(&old.name),
            q(&new.name)
        )));
    }
    for fk in &diff.foreign_keys.added {
        out.push(alter(format!("ADD {}", foreign_key_definition(target, fk))));
    }
    if non_empty(&original.options.comment) != non_empty(&target.options.comment) {
        let comment = non_empty(&target.options.comment).map_or("NULL".to_owned(), literal);
        out.push(format!("COMMENT ON TABLE {table} IS {comment}"));
    }
    for column in &diff.columns {
        let before = column.before.and_then(|b| non_empty(&b.comment));
        let after = non_empty(&column.after.comment);
        if before != after && (column.before.is_some() || after.is_some()) {
            out.push(column_comment(original, &column.after.name, after));
        }
    }
    if diff.table_renamed() {
        out.push(alter(format!("RENAME TO {}", q(&target.table.table))));
    }
    Ok(out)
}

/// PostgreSQL 没有调整字段顺序的语法,新字段只能加在最后。
fn check_order(diff: &Diff) -> CoreResult<()> {
    if diff.existing_reordered() {
        return Err(CoreError::InvalidEdit(
            "PostgreSQL 不能调整已有字段的顺序，请把字段顺序改回原样。".into(),
        ));
    }
    if diff.added_before_existing() {
        return Err(CoreError::InvalidEdit(
            "PostgreSQL 只能把新字段加在最后，请把新字段挪到已有字段后面。".into(),
        ));
    }
    Ok(())
}

/// 留着的表达式索引、部分索引用到了要删的字段:PostgreSQL 删字段会把索引悄悄一起删掉,先拦下来。
fn check_raw_indexes(diff: &Diff) -> CoreResult<()> {
    for index in diff
        .target
        .indexes
        .iter()
        .filter(|i| i.definition.is_some())
    {
        let names = identifiers(index.definition.as_deref().unwrap_or_default());
        if let Some(column) = diff
            .dropped_columns
            .iter()
            .find(|c| names.contains(&c.name.to_lowercase()))
        {
            return Err(CoreError::InvalidEdit(format!(
                "索引「{}」用到了要删除的字段「{}」，删字段会把这个索引一起删掉。请先在索引页删掉它，再删字段。",
                index.name, column.name
            )));
        }
    }
    Ok(())
}

/// 一个原有字段的改动。先去掉旧默认值再改类型(旧默认值可能转不过去),最后设新默认值。
fn alter_column(column: &TargetColumn) -> CoreResult<Vec<String>> {
    let Some(before) = column.before else {
        return Ok(Vec::new());
    };
    let after = column.after;
    if before.generated != after.generated {
        return Err(CoreError::InvalidEdit(format!(
            "PostgreSQL 不能修改已有字段「{}」的生成列表达式，请删掉这个字段后重新添加。",
            after.name
        )));
    }
    let name = q(&after.name);
    let action = |what: String| format!("ALTER COLUMN {name} {what}");
    let mut out = Vec::new();
    let default_changed = before.default != after.default;
    if default_changed && before.default.is_some() {
        out.push(action("DROP DEFAULT".into()));
    }
    if before.auto_increment && !after.auto_increment {
        out.push(action("DROP IDENTITY".into()));
    }
    if before.data_type != after.data_type || before.collation != after.collation {
        let data_type = after.data_type.trim();
        let collate = collate_clause(after);
        out.push(action(format!(
            "TYPE {data_type}{collate} USING {name}::{data_type}"
        )));
    }
    if before.nullable != after.nullable {
        out.push(action(
            if after.nullable {
                "DROP NOT NULL"
            } else {
                "SET NOT NULL"
            }
            .into(),
        ));
    }
    if default_changed && let Some(default) = &after.default {
        out.push(action(format!("SET DEFAULT {}", default.trim())));
    }
    if !before.auto_increment && after.auto_increment {
        out.push(action("ADD GENERATED BY DEFAULT AS IDENTITY".into()));
    }
    Ok(out)
}

/// 排序规则只能加在字符串类型上;类型改成了别的,读出来的排序规则就不再写。
fn collate_clause(column: &ColumnDef) -> String {
    let head = column.data_type.trim().to_ascii_lowercase();
    let textual = [
        "text",
        "char",
        "varchar",
        "character",
        "bpchar",
        "citext",
        "name",
    ]
    .iter()
    .any(|t| {
        head == *t
            || [" ", "(", "["]
                .iter()
                .any(|next| head.starts_with(&format!("{t}{next}")))
    });
    match &column.collation {
        Some(collation) if textual => format!(" COLLATE {collation}"),
        _ => String::new(),
    }
}

fn column_definition(column: &ColumnDef) -> String {
    let mut sql = format!(
        "{} {}{}",
        q(&column.name),
        column.data_type.trim(),
        collate_clause(column)
    );
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
    } else if column.auto_increment {
        sql.push_str(" GENERATED BY DEFAULT AS IDENTITY");
    }
    if !column.nullable {
        sql.push_str(" NOT NULL");
    }
    if let Some(default) = &column.default {
        sql.push_str(&format!(" DEFAULT {}", default.trim()));
    }
    sql
}

fn primary_key(target: &TableDesign, name: &str) -> String {
    let prefix = constraint_prefix(D, name);
    format!(
        "{prefix}PRIMARY KEY ({})",
        column_list(D, target, &target.primary_key)
    )
}

fn foreign_key_definition(target: &TableDesign, fk: &ForeignKeyDef) -> String {
    let referenced = qualified(D, &[ref_schema(fk, target), &fk.ref_table]);
    let columns = if fk.ref_columns.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = fk.ref_columns.iter().map(|c| q(c)).collect();
        format!(" ({})", names.join(", "))
    };
    format!(
        "{}FOREIGN KEY ({}) REFERENCES {referenced}{columns}{}",
        constraint_prefix(D, &fk.name),
        column_list(D, target, &fk.columns),
        actions(fk)
    )
}

fn create_index(target: &TableDesign, index: &IndexDef) -> String {
    create_index_on(&table_name(D, &target.table), index, target)
}

fn create_index_on(table: &str, index: &IndexDef, target: &TableDesign) -> String {
    let unique = if index.unique { "UNIQUE " } else { "" };
    let using = index
        .method
        .as_deref()
        .map_or(String::new(), |m| format!(" USING {m}"));
    let columns: Vec<String> = index
        .columns
        .iter()
        .map(|c| {
            let name = q(name_of(target, &c.column));
            if c.descending {
                format!("{name} DESC")
            } else {
                name
            }
        })
        .collect();
    format!(
        "CREATE {unique}INDEX {} ON {table}{using} ({})",
        q(&index.name),
        columns.join(", ")
    )
}

fn column_comment(design: &TableDesign, column: &str, comment: Option<&str>) -> String {
    let target = qualified(D, &[schema(design), &design.table.table, column]);
    let comment = comment.map_or("NULL".to_owned(), literal);
    format!("COMMENT ON COLUMN {target} IS {comment}")
}

fn literal(text: &str) -> String {
    string_literal(D, false, text)
}
