use super::test_support::*;
use super::*;

const S: Dialect = Dialect::Sqlite;

fn notes() -> TableDesign {
    let mut title = not_null(col("title", "TEXT"));
    title.default = Some("''".into());
    title.collation = Some("NOCASE".into());
    TableDesign {
        primary_key: vec![cid("id")],
        indexes: vec![index("idx_title", &["title"])],
        foreign_keys: vec![fk("fk_tag", &["tag_id"], "tags", &["id"])],
        ..design(
            S,
            "notes",
            vec![
                ColumnDef {
                    auto_increment: true,
                    ..col("id", "INTEGER")
                },
                title,
                col("body", "TEXT"),
                col("tag_id", "INTEGER"),
            ],
        )
    }
}

fn rebuild_context() -> DdlContext {
    DdlContext {
        referencing_tables: vec!["attachments".into()],
        ..DdlContext::new(S)
    }
}

#[test]
fn create_table_with_inline_autoincrement_and_separate_indexes() {
    let mut target = notes();
    let mut unique = index("uk_body", &["body"]);
    unique.unique = true;
    unique.is_constraint = true;
    target.indexes.push(unique);
    target.foreign_keys[0].on_delete = FkAction::Cascade;
    target.checks = vec![CheckDef {
        id: "ck:0".into(),
        name: Some("title_not_empty".into()),
        expression: "length(title) > 0".into(),
    }];
    target.options.without_rowid = true;
    target.options.strict = true;

    let plan = plan_for(S, None, &target);

    assert_eq!(plan.mode, ExecMode::Transaction);
    assert_eq!(
        sqls(&plan),
        [
            "CREATE TABLE \"main\".\"notes\" (\n  \
             \"id\" INTEGER PRIMARY KEY AUTOINCREMENT,\n  \
             \"title\" TEXT NOT NULL DEFAULT '' COLLATE NOCASE,\n  \
             \"body\" TEXT,\n  \
             \"tag_id\" INTEGER,\n  \
             UNIQUE (\"body\"),\n  \
             CONSTRAINT \"fk_tag\" FOREIGN KEY (\"tag_id\") REFERENCES \"tags\" (\"id\") ON DELETE CASCADE,\n  \
             CONSTRAINT \"title_not_empty\" CHECK (length(title) > 0)\n\
             ) WITHOUT ROWID, STRICT",
            "CREATE INDEX \"main\".\"idx_title\" ON \"notes\" (\"title\")",
        ]
    );
}

#[test]
fn composite_key_typeless_column_and_generated_column() {
    let mut total = col("total", "");
    total.generated = Some(Generated {
        expression: "a + b".into(),
        stored: true,
    });
    let target = TableDesign {
        primary_key: vec![cid("a"), cid("b")],
        ..design(S, "pairs", vec![col("a", ""), col("b", "INT"), total])
    };
    assert_eq!(
        sqls(&plan_for(S, None, &target)),
        ["CREATE TABLE \"main\".\"pairs\" (\n  \
          \"a\",\n  \
          \"b\" INT,\n  \
          \"total\" GENERATED ALWAYS AS (a + b) STORED,\n  \
          PRIMARY KEY (\"a\", \"b\")\n\
          )"]
    );
}

#[test]
fn simple_changes_use_alter_table_directly() {
    let original = notes();
    let mut target = notes();
    target.table.table = "memos".into();
    target.columns[2].name = "content".into();
    let mut pinned = not_null(new_col("pinned", "INTEGER"));
    pinned.default = Some("0".into());
    target.columns.push(pinned);
    target.indexes = vec![new_index("idx_content", &["body DESC"])];
    target.indexes[0].columns[0].column = cid("body");

    let plan = plan_for(S, Some(&original), &target);

    assert_eq!(plan.mode, ExecMode::Transaction);
    assert_eq!(
        sqls(&plan),
        [
            "DROP INDEX \"main\".\"idx_title\"",
            "ALTER TABLE \"main\".\"notes\" RENAME COLUMN \"body\" TO \"content\"",
            "ALTER TABLE \"main\".\"notes\" ADD COLUMN \"pinned\" INTEGER NOT NULL DEFAULT 0",
            "CREATE INDEX \"main\".\"idx_content\" ON \"notes\" (\"content\" DESC)",
            "ALTER TABLE \"main\".\"notes\" RENAME TO \"memos\"",
        ]
    );
}

#[test]
fn dropping_a_plain_column_is_direct() {
    let original = notes();
    let mut target = notes();
    target.columns.remove(2);
    assert_eq!(
        sqls(&plan_for(S, Some(&original), &target)),
        ["ALTER TABLE \"main\".\"notes\" DROP COLUMN \"body\""]
    );
}

#[test]
fn changing_a_column_rebuilds_the_table() {
    let mut original = notes();
    original.checks = vec![CheckDef {
        id: "ck:0".into(),
        name: None,
        expression: "length(title) > 0".into(),
    }];
    original.triggers = vec![TriggerDef {
        name: "trg".into(),
        sql: "CREATE TRIGGER trg AFTER INSERT ON notes BEGIN SELECT 1; END".into(),
    }];
    let mut target = original.clone();
    target.columns[1].data_type = "VARCHAR(100)".into();

    let plan = plan(&rebuild_context(), Some(&original), &target).unwrap();

    assert_eq!(plan.mode, ExecMode::Rebuild);
    assert_eq!(
        sqls(&plan),
        [
            "PRAGMA legacy_alter_table = ON",
            "CREATE TABLE \"main\".\"_basalt_new_notes\" (\n  \
             \"id\" INTEGER PRIMARY KEY AUTOINCREMENT,\n  \
             \"title\" VARCHAR(100) NOT NULL DEFAULT '' COLLATE NOCASE,\n  \
             \"body\" TEXT,\n  \
             \"tag_id\" INTEGER,\n  \
             CONSTRAINT \"fk_tag\" FOREIGN KEY (\"tag_id\") REFERENCES \"tags\" (\"id\"),\n  \
             CHECK (length(title) > 0)\n\
             )",
            "INSERT INTO \"main\".\"_basalt_new_notes\" (\"id\", \"title\", \"body\", \"tag_id\") \
             SELECT \"id\", \"title\", \"body\", \"tag_id\" FROM \"main\".\"notes\"",
            "DELETE FROM \"main\".sqlite_sequence WHERE name = '_basalt_new_notes'",
            "INSERT INTO \"main\".sqlite_sequence (name, seq) \
             SELECT '_basalt_new_notes', seq FROM \"main\".sqlite_sequence WHERE name = 'notes'",
            "DROP TABLE \"main\".\"notes\"",
            "ALTER TABLE \"main\".\"_basalt_new_notes\" RENAME TO \"notes\"",
            "PRAGMA legacy_alter_table = OFF",
            "CREATE INDEX \"main\".\"idx_title\" ON \"notes\" (\"title\")",
            "CREATE TRIGGER trg AFTER INSERT ON notes BEGIN SELECT 1; END",
            "PRAGMA \"main\".foreign_key_check(\"notes\")",
            "PRAGMA \"main\".foreign_key_check(\"attachments\")",
        ]
    );
    let checks: Vec<_> = plan
        .statements
        .iter()
        .filter(|s| s.kind == StatementKind::ForeignKeyCheck)
        .collect();
    assert_eq!(checks.len(), 2);
}

#[test]
fn rebuild_maps_renamed_dropped_and_added_columns() {
    let mut original = notes();
    original.columns[0].auto_increment = false;
    let mut target = original.clone();
    target.columns[1].data_type = "VARCHAR(100)".into();
    target.columns[2].name = "content".into();
    target.columns.remove(3);
    target.foreign_keys.clear();
    target.columns.insert(1, new_col("kind", "TEXT"));
    let mut raw = index("idx_lower", &[]);
    raw.definition = Some("CREATE INDEX idx_lower ON notes(lower(title))".into());
    original.indexes.push(raw.clone());
    target.indexes.push(raw);

    let plan = plan_for(S, Some(&original), &target);

    let statements = sqls(&plan);
    assert_eq!(
        statements[2],
        "INSERT INTO \"main\".\"_basalt_new_notes\" (\"id\", \"title\", \"content\") \
         SELECT \"id\", \"title\", \"body\" FROM \"main\".\"notes\""
    );
    assert!(statements[1].contains("\"id\" INTEGER,\n  \"kind\" TEXT,\n"));
    assert!(statements[1].contains("PRIMARY KEY (\"id\")"));
    assert!(statements.contains(&"CREATE INDEX idx_lower ON notes(lower(title))"));
    assert!(!statements.iter().any(|s| s.contains("sqlite_sequence")));
}

type Change = Box<dyn Fn(&mut TableDesign)>;

#[test]
fn these_changes_need_a_rebuild() {
    let cases: Vec<(&str, Change)> = vec![
        (
            "加外键",
            Box::new(|t| t.foreign_keys.push(new_fk("", &["body"], "x", &["id"]))),
        ),
        ("删外键", Box::new(|t| t.foreign_keys.clear())),
        (
            "非空且无默认值的新字段",
            Box::new(|t| t.columns.push(not_null(new_col("x", "TEXT")))),
        ),
        (
            "默认值是表达式的新字段",
            Box::new(|t| {
                let mut c = new_col("x", "TEXT");
                c.default = Some("CURRENT_TIMESTAMP".into());
                t.columns.push(c);
            }),
        ),
        (
            "删主键字段",
            Box::new(|t| {
                t.columns.remove(0);
                t.primary_key.clear();
            }),
        ),
        (
            "新字段插在中间",
            Box::new(|t| t.columns.insert(1, new_col("x", "TEXT"))),
        ),
        ("调换已有字段", Box::new(|t| t.columns.swap(1, 2))),
        (
            "改主键",
            Box::new(|t| {
                t.columns[0].auto_increment = false;
                t.primary_key.push(cid("title"));
            }),
        ),
    ];
    for (label, change) in cases {
        let original = notes();
        let mut target = notes();
        change(&mut target);
        let plan = plan_for(S, Some(&original), &target);
        assert_eq!(plan.mode, ExecMode::Rebuild, "{label}");
    }
}

#[test]
fn dropping_a_unique_constraint_rebuilds() {
    let mut original = notes();
    let mut unique = index("sqlite_autoindex_notes_1", &["body"]);
    unique.unique = true;
    unique.is_constraint = true;
    original.indexes.push(unique);
    let mut target = original.clone();
    target.indexes.pop();
    assert_eq!(
        plan_for(S, Some(&original), &target).mode,
        ExecMode::Rebuild
    );
}

#[test]
fn rebuild_refuses_renames_that_dependents_still_use() {
    let mut original = notes();
    original.triggers = vec![TriggerDef {
        name: "trg".into(),
        sql: "CREATE TRIGGER trg AFTER UPDATE ON notes BEGIN SELECT NEW.body; END".into(),
    }];
    let mut target = original.clone();
    target.columns[1].data_type = "VARCHAR(100)".into();
    target.columns[2].name = "content".into();
    let message = plan_error(S, Some(&original), &target);
    assert!(
        message.contains("trg") && message.contains("body"),
        "{message}"
    );
    assert!(message.contains("分两次"), "{message}");

    // 只改名、不重建时 SQLite 自己会改写触发器,照常放行。
    let mut rename_only = original.clone();
    rename_only.columns[2].name = "content".into();
    assert_eq!(
        plan_for(S, Some(&original), &rename_only).mode,
        ExecMode::Transaction
    );
}

#[test]
fn rebuild_refuses_table_rename_when_views_refer_to_it() {
    let original = notes();
    let mut target = notes();
    target.table.table = "memos".into();
    target.columns[1].data_type = "VARCHAR(100)".into();
    let context = DdlContext {
        schema_objects: vec![SchemaObject {
            kind: "view".into(),
            name: "recent".into(),
            sql: "CREATE VIEW recent AS SELECT * FROM notes".into(),
        }],
        ..DdlContext::new(S)
    };
    let message = plan(&context, Some(&original), &target)
        .unwrap_err()
        .to_string();
    assert!(message.contains("recent"), "{message}");
}

#[test]
fn autoincrement_needs_a_lone_integer_primary_key() {
    let mut target = notes();
    target.columns[0].data_type = "BIGINT".into();
    assert!(plan_error(S, None, &target).contains("INTEGER"));

    let mut composite = notes();
    composite.primary_key.push(cid("title"));
    assert!(plan_error(S, None, &composite).contains("INTEGER"));
}
