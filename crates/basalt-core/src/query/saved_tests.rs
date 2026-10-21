use super::*;

fn query(database: &str, schema: Option<&str>, name: &str) -> SavedQueryRef {
    SavedQueryRef {
        connection_id: "c1".into(),
        database: database.into(),
        schema: schema.map(str::to_owned),
        name: name.into(),
    }
}

#[test]
fn names_must_work_as_file_names() {
    assert_eq!(validate_name("  客户统计  ").unwrap(), "客户统计");
    for bad in [
        "", "   ", ".hidden", "a/b", "a\\b", "a:b", "a\nb", "a?b", "a*b", "a|b", "<a>", "\"a\"",
        "con", "NUL.txt", "com1", "Lpt9",
    ] {
        let error = validate_name(bad).unwrap_err();
        assert_eq!(error.code(), "invalid_name", "{bad:?}");
    }
    assert!(validate_name(&"a".repeat(MAX_NAME_CHARS + 1)).is_err());
    assert!(validate_name(&"a".repeat(MAX_NAME_CHARS)).is_ok());
    // 按字节也要放得进文件名:80 个汉字 = 240 字节。
    assert!(validate_name(&"长".repeat(80)).is_ok());
    assert!(validate_name(&"长".repeat(81)).is_err());
    assert!(validate_name(&sanitize_name(&"长".repeat(200))).is_ok());
}

#[test]
fn sanitize_replaces_what_validate_rejects() {
    assert_eq!(sanitize_name("a/b:c"), "a_b_c");
    assert_eq!(sanitize_name("..secret"), "secret");
    assert_eq!(sanitize_name("  "), "未命名查询");
    assert!(validate_name(&sanitize_name(".\n/")).is_ok());
    assert_eq!(sanitize_name("几点?|全部*"), "几点__全部_");
    assert_eq!(sanitize_name("aux"), "_aux");
    assert!(validate_name(&sanitize_name("aux")).is_ok());
    for fine in ["console", "com0", "com10", "nul 统计", "lpt"] {
        assert!(validate_name(fine).is_ok(), "{fine}");
    }
}

#[test]
fn windows_segments_also_avoid_what_windows_rejects() {
    assert_eq!(encode_segment_for("a?b*c", false), "a?b*c");
    assert_eq!(encode_segment_for("a?b*c", true), "a%3Fb%2Ac");
    assert_eq!(encode_segment_for("db.", true), "db%2E");
    assert_eq!(encode_segment_for("db ", true), "db%20");
    assert_eq!(encode_segment_for("con", true), "%63on");
    assert_eq!(encode_segment_for("con", false), "con");
    assert_eq!(encode_segment_for("shop", true), "shop");
    // 结尾的点在 Windows 上会被悄悄去掉,也要编码。
    assert_eq!(encode_segment_for("..", true), "%2E%2E");
}

#[test]
fn segments_are_percent_encoded_only_where_needed() {
    assert_eq!(encode_segment_for("shop", false), "shop");
    assert_eq!(encode_segment_for("库 名", false), "库 名");
    assert_eq!(encode_segment_for("a/b%c", false), "a%2Fb%25c");
    assert_eq!(encode_segment_for("..", false), "%2E.");
    assert_eq!(encode_segment_for("", false), "%");
    assert_eq!(
        encode_segment(".."),
        encode_segment_for("..", cfg!(windows))
    );
}

#[test]
fn save_list_read_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(dir.path());
    assert!(store.list("c1", "shop", None).unwrap().is_empty());

    store
        .save(&query("shop", None, "b 报表"), "SELECT 2;", false)
        .unwrap();
    let saved = store
        .save(&query("shop", None, " A 统计 "), "SELECT 1;", false)
        .unwrap();
    assert_eq!(saved.name, "A 统计");
    assert!(saved.modified_at.is_some());

    let names: Vec<_> = store
        .list("c1", "shop", None)
        .unwrap()
        .into_iter()
        .map(|q| q.name)
        .collect();
    assert_eq!(names, ["A 统计", "b 报表"], "不分大小写按名字排");
    assert_eq!(
        store.read(&query("shop", None, "A 统计")).unwrap(),
        "SELECT 1;"
    );
    assert!(store.exists(&query("shop", None, "b 报表")));

    store.delete(&query("shop", None, "b 报表")).unwrap();
    store.delete(&query("shop", None, "b 报表")).unwrap();
    assert!(!store.exists(&query("shop", None, "b 报表")));
}

#[test]
fn same_name_needs_explicit_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(dir.path());
    let target = query("shop", None, "q");
    store.save(&target, "SELECT 1", false).unwrap();
    assert_eq!(
        store.save(&target, "SELECT 2", false).unwrap_err().code(),
        "already_exists"
    );
    store.save(&target, "SELECT 2", true).unwrap();
    assert_eq!(store.read(&target).unwrap(), "SELECT 2");
}

#[test]
fn postgres_queries_live_under_their_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(dir.path());
    store
        .save(&query("folio", Some("public"), "q"), "SELECT 1", false)
        .unwrap();
    assert_eq!(store.list("c1", "folio", Some("public")).unwrap().len(), 1);
    assert!(store.list("c1", "folio", Some("other")).unwrap().is_empty());
    assert!(dir.path().join("c1/folio/public/q.sql").is_file());
}

#[test]
fn odd_database_names_stay_inside_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(dir.path().join("queries"));
    store
        .save(&query("../escape", None, "q"), "SELECT 1", false)
        .unwrap();
    assert!(dir.path().join("queries/c1/%2E.%2Fescape/q.sql").is_file());
    assert_eq!(store.list("c1", "../escape", None).unwrap().len(), 1);
}

#[test]
fn only_sql_files_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(dir.path());
    store
        .save(&query("shop", None, "q"), "SELECT 1", false)
        .unwrap();
    let folder = dir.path().join("c1/shop");
    std::fs::write(folder.join(".DS_Store"), "").unwrap();
    std::fs::write(folder.join("note.txt"), "").unwrap();
    std::fs::write(folder.join(".hidden.sql"), "").unwrap();
    std::fs::create_dir(folder.join("dir.sql")).unwrap();
    let names: Vec<_> = store
        .list("c1", "shop", None)
        .unwrap()
        .into_iter()
        .map(|q| q.name)
        .collect();
    assert_eq!(names, ["q"]);
}

#[test]
fn reading_reports_missing_and_non_utf8_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(dir.path());
    assert_eq!(
        store.read(&query("shop", None, "nope")).unwrap_err().code(),
        "not_found"
    );
    std::fs::create_dir_all(dir.path().join("c1/shop")).unwrap();
    std::fs::write(dir.path().join("c1/shop/bin.sql"), [0xff, 0xfe]).unwrap();
    assert_eq!(
        store.read(&query("shop", None, "bin")).unwrap_err().code(),
        "unsupported"
    );
    assert_eq!(
        store.read(&query("shop", None, "a/b")).unwrap_err().code(),
        "invalid_name"
    );
}
