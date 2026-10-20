//! SQLite 的 DDL。能直接 ALTER 的(改表名、字段改名、加删字段、建删索引)就直接做;
//! 其余按官方推荐的流程重建整张表:建新表 → 复制数据 → 删旧表 → 改名 → 重建索引和触发器,
//! 全程在一个事务里,执行前关掉外键约束,提交前做外键检查(见驱动里的执行)。
//! <https://www.sqlite.org/lang_altertable.html#otheralter>

use super::diff::Diff;
use super::render::{actions, body, column_list, constraint_prefix, name_of, names};
use super::sqlite_rebuild;
use super::{
    CheckDef, ColumnDef, DdlContext, DdlPlan, DdlStatement, ExecMode, ForeignKeyDef, IndexDef,
    TableDesign, TableOptions,
};
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, quote_ident, table_name};

pub(super) const D: Dialect = Dialect::Sqlite;

pub(super) fn q(name: &str) -> String {
    quote_ident(D, name)
}

pub(super) fn plan(
    context: &DdlContext,
    original: Option<&TableDesign>,
    target: &TableDesign,
) -> CoreResult<DdlPlan> {
    let Some(original) = original else {
        return Ok(transaction(create_table(target)));
    };
    let diff = Diff::new(D, original, target);
    if needs_rebuild(&diff) {
        if !context.rebuild_blockers.is_empty() {
            return Err(CoreError::InvalidEdit(format!(
                "这次修改要重建整张表，但表定义里有设计器保留不了的写法（{}），重建会把它们丢掉。请在 SQL 编辑器里手动修改。",
                context.rebuild_blockers.join("、")
            )));
        }
        sqlite_rebuild::check_dependents(context, &diff)?;
        Ok(sqlite_rebuild::rebuild(context, &diff))
    } else {
        Ok(transaction(direct(&diff)))
    }
}

fn transaction(statements: Vec<String>) -> DdlPlan {
    DdlPlan {
        mode: ExecMode::Transaction,
        statements: statements.into_iter().map(DdlStatement::execute).collect(),
    }
}

fn create_table(target: &TableDesign) -> Vec<String> {
    let constraint = |index: &IndexDef| index.is_constraint;
    let table = table_name(D, &target.table);
    let mut statements = vec![table_definition(
        &table,
        target,
        &constraint,
        &target.checks,
        &target.options,
    )];
    statements.extend(
        target
            .indexes
            .iter()
            .filter(|i| !i.is_constraint)
            .map(|i| create_index(target, i)),
    );
    statements
}

/// 能直接 ALTER 的改动。先删索引和字段,再改名(删掉的名字可以被改名占用),最后加字段、建索引、改表名。
fn direct(diff: &Diff) -> Vec<String> {
    let table = table_name(D, &diff.original.table);
    let alter = |action: String| format!("ALTER TABLE {table} {action}");
    let schema = &diff.original.table.database;
    let mut out = Vec::new();
    let indexes = &diff.indexes;
    for index in indexes
        .dropped
        .iter()
        .copied()
        .chain(indexes.renamed.iter().map(|(old, _)| *old))
    {
        out.push(format!("DROP INDEX {}.{}", q(schema), q(&index.name)));
    }
    for column in &diff.dropped_columns {
        out.push(alter(format!("DROP COLUMN {}", q(&column.name))));
    }
    for (from, to) in diff.rename_steps() {
        out.push(alter(format!("RENAME COLUMN {} TO {}", q(&from), q(&to))));
    }
    for column in diff.columns.iter().filter(|c| c.before.is_none()) {
        out.push(alter(format!(
            "ADD COLUMN {}",
            column_definition(column.after, false)
        )));
    }
    let on_table = TableDesign {
        table: diff.original.table.clone(),
        ..diff.target.clone()
    };
    for index in indexes
        .added
        .iter()
        .copied()
        .chain(indexes.renamed.iter().map(|(_, new)| *new))
    {
        out.push(create_index(&on_table, index));
    }
    if diff.table_renamed() {
        out.push(alter(format!("RENAME TO {}", q(&diff.target.table.table))));
    }
    out
}

fn needs_rebuild(diff: &Diff) -> bool {
    let original = diff.original;
    let constraint_changed = diff
        .indexes
        .dropped
        .iter()
        .chain(diff.indexes.renamed.iter().map(|(old, _)| old))
        .any(|i| i.is_constraint);
    let column_changed = diff.columns.iter().any(|c| {
        c.before.is_some_and(|b| {
            let a = c.after;
            b.data_type != a.data_type
                || b.nullable != a.nullable
                || b.default != a.default
                || b.auto_increment != a.auto_increment
                || b.collation != a.collation
                || b.generated != a.generated
        })
    });
    let keyed = |column: &ColumnDef| {
        original.primary_key.contains(&column.id)
            || original
                .foreign_keys
                .iter()
                .any(|f| f.columns.contains(&column.id))
            || original
                .indexes
                .iter()
                .any(|i| i.is_constraint && i.columns.iter().any(|c| c.column == column.id))
    };
    column_changed
        || diff.primary_key_changed
        || !diff.foreign_keys.is_empty()
        || constraint_changed
        || diff.existing_reordered()
        || diff.added_before_existing()
        || diff.dropped_columns.iter().any(|c| keyed(c))
        || diff
            .columns
            .iter()
            .any(|c| c.before.is_none() && !addable(diff.target, c.after))
}

/// ALTER TABLE ADD COLUMN 的限制:不能是主键、不能自增、默认值必须是常量,
/// 非空就得有非空的默认值,STORED 生成列不能加。
fn addable(target: &TableDesign, column: &ColumnDef) -> bool {
    if target.primary_key.contains(&column.id) || column.auto_increment {
        return false;
    }
    if column.generated.as_ref().is_some_and(|g| g.stored) {
        return false;
    }
    match column.default.as_deref().map(str::trim) {
        None => column.nullable,
        Some(default) if default.eq_ignore_ascii_case("NULL") => column.nullable,
        Some(default) => is_constant(default),
    }
}

fn is_constant(text: &str) -> bool {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let quoted = |prefix: &str| {
        text.len() > prefix.len() && text.starts_with(prefix) && text.ends_with('\'')
    };
    unsigned.parse::<f64>().is_ok()
        || quoted("'")
        || quoted("X'")
        || quoted("x'")
        || text.eq_ignore_ascii_case("TRUE")
        || text.eq_ignore_ascii_case("FALSE")
}

pub(super) fn table_definition(
    name: &str,
    design: &TableDesign,
    is_constraint: &dyn Fn(&IndexDef) -> bool,
    checks: &[CheckDef],
    options: &TableOptions,
) -> String {
    let inline_key = match design.primary_key.as_slice() {
        [id] => design
            .columns
            .iter()
            .find(|c| &c.id == id && c.auto_increment),
        _ => None,
    };
    let mut lines: Vec<String> = design
        .columns
        .iter()
        .map(|c| column_definition(c, inline_key.is_some_and(|k| k.id == c.id)))
        .collect();
    if inline_key.is_none() && !design.primary_key.is_empty() {
        let prefix = constraint_prefix(D, design.primary_key_name.as_deref().unwrap_or_default());
        lines.push(format!(
            "{prefix}PRIMARY KEY ({})",
            column_list(D, design, &design.primary_key)
        ));
    }
    for index in design.indexes.iter().filter(|i| is_constraint(i)) {
        let ids: Vec<String> = index.columns.iter().map(|c| c.column.clone()).collect();
        lines.push(format!("UNIQUE ({})", column_list(D, design, &ids)));
    }
    lines.extend(
        design
            .foreign_keys
            .iter()
            .map(|f| foreign_key_definition(design, f)),
    );
    for check in checks {
        let prefix = constraint_prefix(D, check.name.as_deref().unwrap_or_default());
        lines.push(format!("{prefix}CHECK ({})", check.expression));
    }
    let flags: Vec<&str> = [
        options.without_rowid.then_some("WITHOUT ROWID"),
        options.strict.then_some("STRICT"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let suffix = if flags.is_empty() {
        String::new()
    } else {
        format!(" {}", flags.join(", "))
    };
    format!("CREATE TABLE {name} {}{suffix}", body(&lines))
}

fn column_definition(column: &ColumnDef, inline_key: bool) -> String {
    let mut sql = q(&column.name);
    let data_type = column.data_type.trim();
    if !data_type.is_empty() {
        sql.push_str(&format!(" {data_type}"));
    }
    if inline_key {
        sql.push_str(" PRIMARY KEY AUTOINCREMENT");
    }
    if !column.nullable {
        sql.push_str(" NOT NULL");
    }
    if let Some(default) = &column.default {
        sql.push_str(&format!(" DEFAULT {}", default.trim()));
    }
    if let Some(collation) = &column.collation {
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
    sql
}

/// SQLite 的外键只能引用同一个库里的表,表名不带库名。
fn foreign_key_definition(design: &TableDesign, fk: &ForeignKeyDef) -> String {
    let columns = if fk.ref_columns.is_empty() {
        String::new()
    } else {
        format!(" ({})", names(D, fk.ref_columns.iter().map(String::as_str)))
    };
    format!(
        "{}FOREIGN KEY ({}) REFERENCES {}{columns}{}",
        constraint_prefix(D, &fk.name),
        column_list(D, design, &fk.columns),
        q(&fk.ref_table),
        actions(fk)
    )
}

/// 索引名带库名,表名不带:`CREATE INDEX "main"."i" ON "t" (…)`。
pub(super) fn create_index(design: &TableDesign, index: &IndexDef) -> String {
    let unique = if index.unique { "UNIQUE " } else { "" };
    let columns: Vec<String> = index
        .columns
        .iter()
        .map(|c| {
            let name = q(name_of(design, &c.column));
            if c.descending {
                format!("{name} DESC")
            } else {
                name
            }
        })
        .collect();
    format!(
        "CREATE {unique}INDEX {}.{} ON {} ({})",
        q(&design.table.database),
        q(&index.name),
        q(&design.table.table),
        columns.join(", ")
    )
}
