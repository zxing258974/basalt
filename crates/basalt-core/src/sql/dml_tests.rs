//! dml.rs 的单测:语句形状、顺序与拒绝条件。

use super::*;
use crate::sql::test_support::*;
use crate::value::CellValue;

#[test]
fn mysql_update_binds_typed_key_and_converted_values() {
    let (table, columns) = mysql_users();
    let statement = one(
        Dialect::Mysql,
        &table,
        &columns,
        RowChange::Update {
            key: key(&[("id", CellValue::Int(7))]),
            values: values(&[
                ("age", Some(" 42 ")),
                ("name", Some("Bob")),
                ("visits", Some("18446744073709551615")),
                ("balance", Some("-12.50")),
                ("score", Some("0.5")),
                ("flags", Some("5")),
            ]),
        },
    );
    // SET 按表里的列顺序排,与 map 的顺序无关。
    assert_eq!(
        statement.sql(),
        "UPDATE `shop`.`users` SET `name` = ?, `age` = ?, `visits` = ?, `balance` = ?, \
         `score` = ?, `flags` = ? WHERE `id` = ?"
    );
    assert_eq!(
        statement.params(),
        [
            BindValue::Text("Bob".into()),
            BindValue::Int(42),
            BindValue::UInt(u64::MAX),
            BindValue::Text("-12.50".into()),
            BindValue::Float(0.5),
            BindValue::UInt(5),
            BindValue::Int(7),
        ]
    );
    assert_eq!(
        statement.preview(),
        "UPDATE `shop`.`users` SET `name` = 'Bob', `age` = 42, `visits` = 18446744073709551615, \
         `balance` = '-12.50', `score` = 0.5, `flags` = 5 WHERE `id` = 7"
    );
}

#[test]
fn postgres_passes_text_and_casts_to_the_column_type() {
    let (table, columns) = pg_users();
    let statement = one(
        Dialect::Postgres,
        &table,
        &columns,
        RowChange::Update {
            key: key(&[("id", CellValue::Int(7))]),
            values: values(&[
                ("name", Some("Bob")),
                ("age", Some("42")),
                ("tags", Some("{a,b}")),
            ]),
        },
    );
    assert_eq!(
        statement.sql(),
        "UPDATE \"public\".\"users\" SET \"name\" = $1, \"age\" = $2::integer, \
         \"tags\" = $3::text[] WHERE \"id\" = $4::integer"
    );
    assert_eq!(
        statement.params(),
        [
            BindValue::Text("Bob".into()),
            BindValue::Text("42".into()),
            BindValue::Text("{a,b}".into()),
            BindValue::Text("7".into()),
        ]
    );
    assert_eq!(
        statement.preview(),
        "UPDATE \"public\".\"users\" SET \"name\" = 'Bob', \"age\" = '42'::integer, \
         \"tags\" = '{a,b}'::text[] WHERE \"id\" = '7'::integer"
    );
}

#[test]
fn null_values_and_null_keys() {
    let table = TableRef {
        database: "main".into(),
        schema: None,
        table: "codes".into(),
    };
    // SQLite 的非 INTEGER 主键允许 NULL,定位时要用 IS NULL。
    let columns = [
        ColumnInfo {
            is_primary_key: true,
            ..ColumnInfo::new("code", "TEXT")
        },
        ColumnInfo::new("note", "TEXT"),
    ];
    let statement = one(
        Dialect::Sqlite,
        &table,
        &columns,
        RowChange::Update {
            key: key(&[("code", CellValue::Null)]),
            values: values(&[("note", None)]),
        },
    );
    assert_eq!(
        statement.sql(),
        "UPDATE \"main\".\"codes\" SET \"note\" = ? WHERE \"code\" IS NULL"
    );
    assert_eq!(statement.params(), [BindValue::Null]);
    assert_eq!(
        statement.preview(),
        "UPDATE \"main\".\"codes\" SET \"note\" = NULL WHERE \"code\" IS NULL"
    );
}

#[test]
fn insert_lists_only_given_columns_and_falls_back_to_defaults() {
    let (table, columns) = mysql_users();
    let statement = one(
        Dialect::Mysql,
        &table,
        &columns,
        RowChange::Insert {
            values: values(&[("name", Some("A")), ("age", None)]),
        },
    );
    assert_eq!(
        statement.sql(),
        "INSERT INTO `shop`.`users` (`name`, `age`) VALUES (?, ?)"
    );
    assert_eq!(
        statement.preview(),
        "INSERT INTO `shop`.`users` (`name`, `age`) VALUES ('A', NULL)"
    );

    let empty = RowChange::Insert {
        values: values(&[]),
    };
    assert_eq!(
        one(Dialect::Mysql, &table, &columns, empty.clone()).sql(),
        "INSERT INTO `shop`.`users` () VALUES ()"
    );
    let (pg_table, pg_columns) = pg_users();
    assert_eq!(
        one(Dialect::Postgres, &pg_table, &pg_columns, empty).sql(),
        "INSERT INTO \"public\".\"users\" DEFAULT VALUES"
    );
}

#[test]
fn delete_by_composite_key_in_table_column_order() {
    let table = TableRef {
        database: "shop".into(),
        schema: None,
        table: "order_items".into(),
    };
    let columns = [
        pk("order_id", "int"),
        pk("line", "int"),
        ColumnInfo::new("sku", "varchar(20)"),
    ];
    let statement = one(
        Dialect::Mysql,
        &table,
        &columns,
        RowChange::Delete {
            key: key(&[
                ("line", CellValue::Int(2)),
                ("order_id", CellValue::UInt(1)),
            ]),
        },
    );
    assert_eq!(
        statement.sql(),
        "DELETE FROM `shop`.`order_items` WHERE `order_id` = ? AND `line` = ?"
    );
    assert_eq!(statement.params(), [BindValue::UInt(1), BindValue::Int(2)]);
}

#[test]
fn statements_run_deletes_then_updates_then_inserts() {
    let (table, columns) = mysql_users();
    let changes = [
        RowChange::Insert {
            values: values(&[("id", Some("5"))]),
        },
        RowChange::Update {
            key: key(&[("id", CellValue::Int(1))]),
            values: values(&[("name", Some("x"))]),
        },
        RowChange::Update {
            key: key(&[("id", CellValue::Int(2))]),
            values: values(&[]),
        },
        RowChange::Delete {
            key: key(&[("id", CellValue::Int(5))]),
        },
    ];
    let statements = build_changes(Dialect::Mysql, &table, &columns, &changes).unwrap();
    let verbs: Vec<String> = statements
        .iter()
        .map(|s| s.sql().split(' ').next().unwrap().to_owned())
        .collect();
    assert_eq!(verbs, ["DELETE", "UPDATE", "INSERT"], "空的更新应被跳过");
}

#[test]
fn refuses_tables_without_primary_key() {
    let table = TableRef {
        database: "shop".into(),
        schema: None,
        table: "log".into(),
    };
    let columns = [ColumnInfo::new("line", "text")];
    let change = RowChange::Insert {
        values: values(&[("line", Some("x"))]),
    };
    let error = build_changes(Dialect::Mysql, &table, &columns, &[change]).unwrap_err();
    assert_eq!(error.code(), "read_only");
}

#[test]
fn rejects_unknown_generated_and_partial_key_columns() {
    let (table, columns) = mysql_users();
    let cases = [
        RowChange::Update {
            key: key(&[("id", CellValue::Int(1))]),
            values: values(&[("nope", Some("1"))]),
        },
        RowChange::Update {
            key: key(&[("id", CellValue::Int(1))]),
            values: values(&[("label", Some("x"))]),
        },
        RowChange::Delete {
            key: key(&[("name", CellValue::Text("x".into()))]),
        },
        RowChange::Delete { key: key(&[]) },
        RowChange::Delete {
            key: key(&[("id", CellValue::unsupported("GEOMETRY"))]),
        },
    ];
    for change in cases {
        let error = build_changes(
            Dialect::Mysql,
            &table,
            &columns,
            std::slice::from_ref(&change),
        )
        .expect_err(&format!("{change:?}"));
        assert_eq!(error.code(), "invalid_edit", "{change:?}");
    }
}
