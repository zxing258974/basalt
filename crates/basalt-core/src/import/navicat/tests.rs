use super::*;

fn raw(conn_type: &str) -> RawConnection {
    RawConnection {
        name: "库".into(),
        conn_type: conn_type.into(),
        host: "db.example.test".into(),
        port: "3306".into(),
        username: "app".into(),
        ..RawConnection::default()
    }
}

fn reason(target: &SourceTarget) -> &str {
    match target {
        SourceTarget::Unsupported(reason) => reason,
        SourceTarget::Supported { .. } => panic!("应该是不支持"),
    }
}

#[test]
fn maps_navicat_type_names_case_insensitively() {
    assert_eq!(map_type("MySQL"), Ok(DriverKind::Mysql));
    assert_eq!(map_type("MYSQL"), Ok(DriverKind::Mysql));
    assert_eq!(map_type("MariaDB"), Ok(DriverKind::Mariadb));
    assert_eq!(map_type("PostgreSQL"), Ok(DriverKind::Postgres));
    assert_eq!(map_type("POSTGRESQL"), Ok(DriverKind::Postgres));
    assert_eq!(map_type("SQLite"), Ok(DriverKind::Sqlite));
    assert_eq!(map_type("REDIS"), Err("Redis".into()));
    assert_eq!(map_type("SQL Server"), Err("SQL Server".into()));
    assert_eq!(map_type("MongoDB"), Err("MongoDB".into()));
    assert_eq!(map_type("Kingbase"), Err("Kingbase".into()));
    assert_eq!(map_type(""), Err("未知类型".into()));
}

#[test]
fn invalid_port_falls_back_to_the_default_with_a_warning() {
    let (target, warnings) = resolve_target(&RawConnection {
        port: "abc".into(),
        ..raw("PostgreSQL")
    });
    assert!(matches!(
        target,
        SourceTarget::Supported {
            endpoint: Endpoint::Network { port: 5432, .. },
            ..
        }
    ));
    assert_eq!(warnings, ["端口「abc」无效，已改用默认端口 5432"]);

    let (_, warnings) = resolve_target(&RawConnection {
        port: "".into(),
        ..raw("MySQL")
    });
    assert!(warnings.is_empty(), "空端口直接用默认值,不必提示");
}

#[test]
fn incomplete_connections_reuse_the_form_validation_messages() {
    let (target, _) = resolve_target(&RawConnection {
        host: " ".into(),
        username: "".into(),
        ..raw("MySQL")
    });
    assert_eq!(
        reason(&target),
        "连接信息不完整：请填写主机地址；请填写用户名"
    );

    let (target, _) = resolve_target(&RawConnection {
        file_path: "notes.db".into(),
        ..raw("SQLite")
    });
    assert_eq!(reason(&target), "连接信息不完整：请使用绝对路径");
}

#[test]
fn unsupported_connections_carry_no_migration_warnings() {
    let (target, warnings) = resolve_target(&RawConnection {
        uses_tunnel: true,
        ..raw("Oracle")
    });
    assert!(reason(&target).starts_with("Basalt 暂不支持 Oracle"));
    assert!(warnings.is_empty());
}

#[test]
fn ids_are_unique_within_a_source() {
    let sources = into_sources(vec![
        raw("MySQL"),
        raw("MySQL"),
        raw("PostgreSQL"),
        raw("MySQL"),
    ]);
    let ids: Vec<_> = sources.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        ["MySQL:库", "MySQL:库#2", "PostgreSQL:库", "MySQL:库#3"]
    );
}

#[test]
fn location_points_into_the_navicat_support_folder() {
    let location = NavicatLocation::under_home(Path::new("/Users/someone"));
    assert!(
        location
            .conn_plist
            .ends_with("PremiumSoft CyberTech/Navicat CC/Common/conn.plist")
    );
    assert!(
        location
            .vgroup_json
            .ends_with("Navicat Premium/Profiles/vgroup.json")
    );
}

fn fake_home(vgroups: Option<&str>) -> (tempfile::TempDir, NavicatLocation) {
    let home = tempfile::tempdir().unwrap();
    let location = NavicatLocation::under_home(home.path());
    std::fs::create_dir_all(location.conn_plist.parent().unwrap()).unwrap();
    std::fs::write(
        &location.conn_plist,
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/navicat/conn.plist"
        )),
    )
    .unwrap();
    if let Some(json) = vgroups {
        std::fs::create_dir_all(location.vgroup_json.parent().unwrap()).unwrap();
        std::fs::write(&location.vgroup_json, json).unwrap();
    }
    (home, location)
}

#[test]
fn loads_local_navicat_and_applies_groups_when_recognized() {
    let (_home, location) = fake_home(Some(
        r#"{"groups":[{"name":"数仓","children":["分析库"]}]}"#,
    ));
    let sources = location.load().unwrap();
    assert_eq!(sources.len(), 4);
    let analytics = sources.iter().find(|s| s.name == "分析库").unwrap();
    assert_eq!(analytics.group.as_deref(), Some("数仓"));
    assert!(
        sources
            .iter()
            .filter(|s| s.name != "分析库")
            .all(|s| s.group.is_none())
    );
}

#[test]
fn missing_or_broken_group_file_is_ignored() {
    for vgroups in [None, Some("{"), Some("{}")] {
        let (_home, location) = fake_home(vgroups);
        let sources = location.load().unwrap();
        assert_eq!(sources.len(), 4);
        assert!(sources.iter().all(|s| s.group.is_none()));
    }
}

#[test]
fn missing_conn_plist_is_reported_as_not_detected() {
    let home = tempfile::tempdir().unwrap();
    let error = NavicatLocation::under_home(home.path()).load().unwrap_err();
    assert_eq!(error.code(), "storage");
    assert!(
        error.to_string().contains("没有找到本机 Navicat"),
        "{error}"
    );
}

#[test]
fn load_ncx_reads_the_file_and_rejects_non_text_or_oversized_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.ncx");
    std::fs::write(
        &path,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/navicat/export.ncx"
        )),
    )
    .unwrap();
    assert_eq!(load_ncx(&path).unwrap().len(), 9);

    assert_eq!(
        load_ncx(&dir.path().join("missing.ncx"))
            .unwrap_err()
            .code(),
        "storage"
    );

    let binary = dir.path().join("binary.ncx");
    std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x80]).unwrap();
    assert_eq!(load_ncx(&binary).unwrap_err().code(), "unsupported");

    let huge = dir.path().join("huge.ncx");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(MAX_SOURCE_BYTES + 1)
        .unwrap();
    assert_eq!(load_ncx(&huge).unwrap_err().code(), "unsupported");
}
