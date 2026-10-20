//! sql 模块测试共用的表结构与构造函数。

use std::collections::BTreeMap;

use super::Dialect;
use super::dml::{Statement, build_changes};
use crate::table::{ColumnInfo, RowChange, TableRef, ValueType};
use crate::value::CellValue;

pub(super) fn column(name: &str, data_type: &str, value_type: ValueType) -> ColumnInfo {
    ColumnInfo {
        value_type,
        ..ColumnInfo::new(name, data_type)
    }
}

pub(super) fn pk(name: &str, data_type: &str) -> ColumnInfo {
    ColumnInfo {
        is_primary_key: true,
        nullable: false,
        ..column(name, data_type, ValueType::Integer { unsigned: false })
    }
}

pub(super) const INT: ValueType = ValueType::Integer { unsigned: false };

pub(super) fn mysql_users() -> (TableRef, Vec<ColumnInfo>) {
    let table = TableRef {
        database: "shop".into(),
        schema: None,
        table: "users".into(),
    };
    let columns = vec![
        pk("id", "int"),
        column("name", "varchar(50)", ValueType::Text),
        column("age", "int", INT),
        column(
            "visits",
            "bigint unsigned",
            ValueType::Integer { unsigned: true },
        ),
        column("balance", "decimal(10,2)", ValueType::Decimal),
        column("score", "double", ValueType::Float),
        column("flags", "bit(8)", ValueType::Bit),
        column("avatar", "blob", ValueType::Binary),
        ColumnInfo {
            is_generated: true,
            ..column("label", "varchar(60)", ValueType::Text)
        },
    ];
    (table, columns)
}

pub(super) fn pg_users() -> (TableRef, Vec<ColumnInfo>) {
    let table = TableRef {
        database: "app".into(),
        schema: Some("public".into()),
        table: "users".into(),
    };
    let cast = |name: &str, data_type: &str, cast: &str, value_type| ColumnInfo {
        write_cast: Some(cast.into()),
        ..column(name, data_type, value_type)
    };
    let columns = vec![
        ColumnInfo {
            write_cast: Some("integer".into()),
            ..pk("id", "integer")
        },
        column("name", "character varying(50)", ValueType::Text),
        cast("age", "integer", "integer", INT),
        cast("photo", "bytea", "bytea", ValueType::Binary),
        cast("tags", "text[]", "text[]", ValueType::Text),
    ];
    (table, columns)
}

pub(super) fn key(pairs: &[(&str, CellValue)]) -> BTreeMap<String, CellValue> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

pub(super) fn values(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.map(str::to_owned)))
        .collect()
}

pub(super) fn one(
    dialect: Dialect,
    table: &TableRef,
    columns: &[ColumnInfo],
    change: RowChange,
) -> Statement {
    let mut statements = build_changes(dialect, table, columns, &[change]).unwrap();
    assert_eq!(statements.len(), 1);
    statements.remove(0)
}
