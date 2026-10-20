use super::*;

#[test]
fn picks_up_column_details_checks_and_table_flags() {
    let sql = r#"CREATE TABLE "notes" (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        "title" TEXT NOT NULL DEFAULT 'a,b' COLLATE NOCASE CHECK (length(title) > 0),
        [body] VARCHAR(10, 2) DEFAULT (lower('X')), -- 注释里的 CHECK (x) 不算
        total INT GENERATED ALWAYS AS (id * 2) STORED,
        half AS (id / 2),
        `tag` INT CONSTRAINT tag_positive CHECK (tag > 0) REFERENCES tags(id),
        CONSTRAINT fk_tag FOREIGN KEY (tag, "Title") REFERENCES tags (id, name),
        UNIQUE (title),
        CHECK (id <> 0) /* 末尾的 ) 也不算 */
    ) WITHOUT ROWID, STRICT"#;

    let table = parse_create_table(sql).expect("应能解析");

    let names: Vec<_> = table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "title", "body", "total", "half", "tag"]);
    assert!(table.columns[0].autoincrement);
    assert_eq!(table.columns[1].collation.as_deref(), Some("NOCASE"));
    assert_eq!(
        table.columns[3].generated,
        Some(Generated {
            expression: "id * 2".into(),
            stored: true
        })
    );
    assert_eq!(
        table.columns[4].generated,
        Some(Generated {
            expression: "id / 2".into(),
            stored: false
        })
    );
    assert_eq!(
        table.checks,
        [
            (None, "length(title) > 0".to_owned()),
            (Some("tag_positive".to_owned()), "tag > 0".to_owned()),
            (None, "id <> 0".to_owned()),
        ]
    );
    assert_eq!(
        table.foreign_key_names,
        [(
            vec!["tag".to_owned(), "title".to_owned()],
            "fk_tag".to_owned()
        )]
    );
    assert!(table.without_rowid && table.strict);
}

#[test]
fn plain_table_has_no_flags_and_unparseable_sql_is_none() {
    let table = parse_create_table("CREATE TABLE t(a, b)").unwrap();
    assert_eq!(table.columns.len(), 2);
    assert!(!table.without_rowid && !table.strict && table.checks.is_empty());
    assert_eq!(parse_create_table("CREATE TABLE t AS SELECT 1"), None);
    assert_eq!(parse_create_table("CREATE TABLE t (a"), None);
}

#[test]
fn identifiers_are_lowercased_and_skip_strings_and_comments() {
    let ids = identifiers(r#"SELECT NEW."Body", 'title' FROM [Notes] -- tag"#);
    assert_eq!(ids, ["select", "new", "body", "from", "notes"]);
}

#[test]
fn doubled_quotes_inside_identifiers_and_strings() {
    let table =
        parse_create_table(r#"CREATE TABLE t ("a""b" TEXT DEFAULT 'it''s', `c``d` INT)"#).unwrap();
    let names: Vec<_> = table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["a\"b", "c`d"]);
}
