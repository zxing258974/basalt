//! 设计器单测共用的构造函数。字段、索引在测试里按名字引用,id 由名字推出。

use super::*;

pub(super) fn col(name: &str, data_type: &str) -> ColumnDef {
    ColumnDef {
        id: existing_id(COLUMN_ID_PREFIX, name),
        name: name.into(),
        data_type: data_type.into(),
        nullable: true,
        default: None,
        auto_increment: false,
        comment: None,
        collation: None,
        on_update: None,
        generated: None,
    }
}

/// 设计器里新加的字段:id 不带 `col:` 前缀。
pub(super) fn new_col(name: &str, data_type: &str) -> ColumnDef {
    ColumnDef {
        id: format!("new:{name}"),
        ..col(name, data_type)
    }
}

pub(super) fn not_null(column: ColumnDef) -> ColumnDef {
    ColumnDef {
        nullable: false,
        ..column
    }
}

pub(super) fn cid(name: &str) -> String {
    existing_id(COLUMN_ID_PREFIX, name)
}

pub(super) fn table_ref(dialect: Dialect, table: &str) -> TableRef {
    match dialect {
        Dialect::Mysql => TableRef {
            database: "shop".into(),
            schema: None,
            table: table.into(),
        },
        Dialect::Postgres => TableRef {
            database: "shop".into(),
            schema: Some("public".into()),
            table: table.into(),
        },
        Dialect::Sqlite => TableRef {
            database: "main".into(),
            schema: None,
            table: table.into(),
        },
    }
}

pub(super) fn design(dialect: Dialect, table: &str, columns: Vec<ColumnDef>) -> TableDesign {
    TableDesign {
        table: table_ref(dialect, table),
        columns,
        primary_key: Vec::new(),
        primary_key_name: None,
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
        triggers: Vec::new(),
        options: TableOptions::default(),
        version: String::new(),
    }
}

/// 列名后缀 ` DESC` 表示降序。
pub(super) fn index(name: &str, columns: &[&str]) -> IndexDef {
    IndexDef {
        id: existing_id(INDEX_ID_PREFIX, name),
        name: name.into(),
        columns: columns
            .iter()
            .map(|c| {
                let (column, descending) = match c.strip_suffix(" DESC") {
                    Some(column) => (column, true),
                    None => (*c, false),
                };
                IndexColumn {
                    column: cid(column),
                    descending,
                    length: None,
                }
            })
            .collect(),
        unique: false,
        method: None,
        definition: None,
        is_constraint: false,
    }
}

pub(super) fn new_index(name: &str, columns: &[&str]) -> IndexDef {
    IndexDef {
        id: format!("new:{name}"),
        ..index(name, columns)
    }
}

pub(super) fn fk(
    name: &str,
    columns: &[&str],
    ref_table: &str,
    ref_columns: &[&str],
) -> ForeignKeyDef {
    ForeignKeyDef {
        id: existing_id(FOREIGN_KEY_ID_PREFIX, name),
        name: name.into(),
        columns: columns.iter().map(|c| cid(c)).collect(),
        ref_schema: None,
        ref_table: ref_table.into(),
        ref_columns: ref_columns.iter().map(|c| (*c).to_owned()).collect(),
        on_update: FkAction::NoAction,
        on_delete: FkAction::NoAction,
    }
}

pub(super) fn new_fk(
    name: &str,
    columns: &[&str],
    ref_table: &str,
    ref_columns: &[&str],
) -> ForeignKeyDef {
    ForeignKeyDef {
        id: format!("new:{name}"),
        ..fk(name, columns, ref_table, ref_columns)
    }
}

pub(super) fn sqls(plan: &DdlPlan) -> Vec<&str> {
    plan.statements.iter().map(|s| s.sql.as_str()).collect()
}

pub(super) fn plan_for(
    dialect: Dialect,
    original: Option<&TableDesign>,
    target: &TableDesign,
) -> DdlPlan {
    plan(&DdlContext::new(dialect), original, target).expect("应能生成 DDL")
}

pub(super) fn plan_error(
    dialect: Dialect,
    original: Option<&TableDesign>,
    target: &TableDesign,
) -> String {
    plan(&DdlContext::new(dialect), original, target)
        .expect_err("应当拒绝")
        .to_string()
}
