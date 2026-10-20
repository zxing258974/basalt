//! 表结构设计集成测试共用的构造与执行辅助。
#![allow(dead_code)]

use basalt_core::CoreResult;
use basalt_core::design::{
    ColumnDef, DdlOutcome, ForeignKeyDef, IndexColumn, IndexDef, TableDesign,
};
use basalt_core::session::Session;
use basalt_core::table::TableRef;

/// 设计器里新加的字段。
pub fn new_column(name: &str, data_type: &str, nullable: bool) -> ColumnDef {
    ColumnDef {
        id: format!("new:{name}"),
        name: name.into(),
        data_type: data_type.into(),
        nullable,
        default: None,
        auto_increment: false,
        comment: None,
        collation: None,
        on_update: None,
        generated: None,
    }
}

pub fn id_of(design: &TableDesign, name: &str) -> String {
    design
        .columns
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("没有字段 {name}"))
        .id
        .clone()
}

pub fn column_mut<'a>(design: &'a mut TableDesign, name: &str) -> &'a mut ColumnDef {
    design
        .columns
        .iter_mut()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("没有字段 {name}"))
}

pub fn column_names(design: &TableDesign) -> Vec<&str> {
    design.columns.iter().map(|c| c.name.as_str()).collect()
}

pub fn new_index(
    design: &TableDesign,
    name: &str,
    columns: &[(&str, bool)],
    unique: bool,
) -> IndexDef {
    IndexDef {
        id: format!("new:{name}"),
        name: name.into(),
        columns: columns
            .iter()
            .map(|(column, descending)| IndexColumn {
                column: id_of(design, column),
                descending: *descending,
                length: None,
            })
            .collect(),
        unique,
        method: None,
        definition: None,
        is_constraint: false,
    }
}

pub fn new_foreign_key(
    design: &TableDesign,
    name: &str,
    column: &str,
    ref_table: &str,
    ref_column: &str,
) -> ForeignKeyDef {
    ForeignKeyDef {
        id: format!("new:{name}"),
        name: name.into(),
        columns: vec![id_of(design, column)],
        ref_schema: None,
        ref_table: ref_table.into(),
        ref_columns: vec![ref_column.into()],
        on_update: Default::default(),
        on_delete: Default::default(),
    }
}

/// 和界面一样:先预览,再带着预览去执行。
pub async fn save(
    session: &Session,
    original: Option<&TableRef>,
    target: &TableDesign,
) -> CoreResult<DdlOutcome> {
    let preview = session.preview_design(original, target).await?;
    session
        .apply_design(original, target, Some(&preview.statements))
        .await
}

pub fn index_names(design: &TableDesign) -> Vec<&str> {
    design.indexes.iter().map(|i| i.name.as_str()).collect()
}
