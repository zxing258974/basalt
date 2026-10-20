use super::test_support::*;
use super::*;

fn basic(dialect: Dialect) -> TableDesign {
    TableDesign {
        primary_key: vec![cid("id")],
        ..design(
            dialect,
            "t",
            vec![not_null(col("id", "int")), col("name", "text")],
        )
    }
}

fn rejects(dialect: Dialect, change: impl FnOnce(&mut TableDesign), expected: &str) {
    let mut target = basic(dialect);
    change(&mut target);
    let message = plan_error(dialect, None, &target);
    assert!(
        message.contains(expected),
        "期望包含「{expected}」,实际:{message}"
    );
}

#[test]
fn table_and_columns_need_names_and_types() {
    let m = Dialect::Mysql;
    rejects(m, |t| t.table.table = "  ".into(), "表名");
    rejects(m, |t| t.columns.clear(), "至少");
    rejects(m, |t| t.columns[1].name = String::new(), "第 2 个字段");
    rejects(m, |t| t.columns[1].data_type = " ".into(), "类型");
    rejects(m, |t| t.columns[1].name = "ID".into(), "重复");
    rejects(m, |t| t.columns[1].id = t.columns[0].id.clone(), "重复");
}

#[test]
fn postgres_names_are_case_sensitive_and_sqlite_allows_typeless_columns() {
    let mut pg = basic(Dialect::Postgres);
    pg.columns[1].name = "ID".into();
    assert!(plan(&DdlContext::new(Dialect::Postgres), None, &pg).is_ok());

    let mut sqlite = basic(Dialect::Sqlite);
    sqlite.columns[1].data_type = String::new();
    assert!(plan(&DdlContext::new(Dialect::Sqlite), None, &sqlite).is_ok());
}

#[test]
fn raw_sql_fragments_cannot_smuggle_statements() {
    let m = Dialect::Mysql;
    rejects(
        m,
        |t| t.columns[1].default = Some("1; DROP TABLE t".into()),
        "默认值",
    );
    rejects(m, |t| t.columns[1].data_type = "int)".into(), "类型");
    rejects(
        m,
        |t| {
            t.columns[1].generated = Some(Generated {
                expression: "'a".into(),
                stored: false,
            });
        },
        "生成",
    );
    rejects(
        m,
        |t| t.columns[1].collation = Some("utf8 bin".into()),
        "排序规则",
    );
    rejects(m, |t| t.options.engine = Some("Inno;DB".into()), "引擎");
    // 字符串里的分号不算。
    let mut ok = basic(m);
    ok.columns[1].default = Some("'a;b'".into());
    assert!(plan(&DdlContext::new(m), None, &ok).is_ok());
}

#[test]
fn conflicting_column_attributes() {
    let m = Dialect::Mysql;
    rejects(
        m,
        |t| {
            t.columns[0].auto_increment = true;
            t.columns[0].default = Some("1".into());
        },
        "自增",
    );
    rejects(
        m,
        |t| {
            t.columns[1].default = Some("1".into());
            t.columns[1].generated = Some(Generated {
                expression: "id + 1".into(),
                stored: true,
            });
        },
        "生成列",
    );
    rejects(
        m,
        |t| {
            t.columns[0].auto_increment = true;
            t.columns[1].auto_increment = true;
        },
        "只能有一个",
    );
}

#[test]
fn keys_and_indexes_must_point_at_existing_columns() {
    let m = Dialect::Mysql;
    rejects(m, |t| t.primary_key = vec!["new:x".into()], "主键");
    rejects(m, |t| t.primary_key.push(cid("id")), "主键");
    rejects(m, |t| t.indexes.push(new_index("i", &[])), "至少选一个字段");
    rejects(m, |t| t.indexes.push(new_index("", &["name"])), "索引名");
    rejects(
        m,
        |t| t.indexes = vec![new_index("i", &["name"]), new_index("I", &["id"])],
        "重复",
    );
    rejects(m, |t| t.indexes.push(new_index("i", &["gone"])), "不存在");
    rejects(
        m,
        |t| t.indexes.push(new_index("i", &["name", "name"])),
        "重复",
    );
    rejects(
        m,
        |t| {
            let mut i = new_index("i", &["name"]);
            i.method = Some("BTREE x".into());
            t.indexes.push(i);
        },
        "索引类型",
    );
    rejects(
        m,
        |t| {
            let mut i = new_index("i", &[]);
            i.definition = Some("CREATE INDEX ...".into());
            t.indexes.push(i);
        },
        "不能新建",
    );
}

#[test]
fn foreign_keys_need_matching_columns() {
    let m = Dialect::Mysql;
    rejects(
        m,
        |t| t.foreign_keys.push(new_fk("f", &[], "p", &[])),
        "至少选一个字段",
    );
    rejects(
        m,
        |t| t.foreign_keys.push(new_fk("f", &["id"], " ", &["id"])),
        "引用的表",
    );
    rejects(
        m,
        |t| {
            t.foreign_keys
                .push(new_fk("f", &["id", "name"], "p", &["id"]))
        },
        "个数",
    );
    rejects(
        m,
        |t| t.foreign_keys.push(new_fk("f", &["id"], "p", &[])),
        "个数",
    );
    rejects(
        m,
        |t| t.foreign_keys.push(new_fk("f", &["id"], "p", &[" "])),
        "引用的字段",
    );
    rejects(
        m,
        |t| {
            t.foreign_keys = vec![
                new_fk("f", &["id"], "p", &["id"]),
                new_fk("F", &["id"], "q", &["id"]),
            ]
        },
        "重复",
    );
    // PostgreSQL 可以省略引用字段,表示引用对方的主键。
    let mut pg = basic(Dialect::Postgres);
    pg.foreign_keys.push(new_fk("f", &["id"], "p", &[]));
    assert!(plan(&DdlContext::new(Dialect::Postgres), None, &pg).is_ok());
}

#[test]
fn renaming_cannot_move_the_table_to_another_database() {
    let original = basic(Dialect::Mysql);
    let mut target = basic(Dialect::Mysql);
    target.table.database = "other".into();
    let message = plan_error(Dialect::Mysql, Some(&original), &target);
    assert!(message.contains("库"), "{message}");
}
