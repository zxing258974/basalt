//! 与前端 src/lib/designTypes.ts 的序列化约定:字段名 camelCase,枚举是小写开头的字符串。

use serde_json::json;

use super::*;

#[test]
fn design_from_the_frontend_deserializes_and_round_trips() {
    let sent = json!({
        "table": { "database": "shop", "schema": null, "table": "users" },
        "columns": [{
            "id": "new:1", "name": "id", "dataType": "int", "nullable": false, "default": null,
            "autoIncrement": true, "comment": "主键", "collation": null, "onUpdate": null,
            "generated": { "expression": "1", "stored": true }
        }],
        "primaryKey": ["new:1"],
        "primaryKeyName": null,
        "indexes": [{
            "id": "new:2", "name": "idx", "columns": [{ "column": "new:1", "descending": true, "length": 10 }],
            "unique": true, "method": "HASH", "definition": null, "isConstraint": false
        }],
        "foreignKeys": [{
            "id": "new:3", "name": "", "columns": ["new:1"], "refSchema": "hr", "refTable": "teams",
            "refColumns": ["id"], "onUpdate": "setNull", "onDelete": "cascade"
        }],
        "checks": [{ "id": "ck:0", "name": null, "expression": "id > 0" }],
        "triggers": [{ "name": "t", "sql": "CREATE TRIGGER t" }],
        "options": {
            "engine": "InnoDB", "charset": null, "collation": null, "comment": null,
            "withoutRowid": true, "strict": false
        },
        "version": "0123456789abcdef"
    });
    let design: TableDesign = serde_json::from_value(sent.clone()).unwrap();
    assert!(design.columns[0].auto_increment);
    assert_eq!(design.indexes[0].columns[0].length, Some(10));
    assert_eq!(design.foreign_keys[0].on_update, FkAction::SetNull);
    assert!(design.options.without_rowid);
    assert_eq!(serde_json::to_value(&design).unwrap(), sent);
}

#[test]
fn optional_parts_may_be_left_out() {
    let design: TableDesign = serde_json::from_value(json!({
        "table": { "database": "main", "table": "t" },
        "columns": [{ "id": "new:1", "name": "a", "dataType": "", "nullable": true }]
    }))
    .unwrap();
    assert!(design.primary_key.is_empty() && design.indexes.is_empty());
    assert_eq!(design.options, TableOptions::default());
}

#[test]
fn preview_and_outcome_use_camel_case() {
    let plan = DdlPlan {
        mode: ExecMode::Rebuild,
        statements: vec![DdlStatement::execute("SELECT 1")],
    };
    assert_eq!(
        serde_json::to_value(plan.preview()).unwrap(),
        json!({ "statements": ["SELECT 1"], "mode": "rebuild" })
    );
    assert_eq!(
        serde_json::to_value(ExecMode::Autocommit).unwrap(),
        json!("autocommit")
    );
}
