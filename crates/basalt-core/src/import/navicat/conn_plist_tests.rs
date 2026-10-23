use super::*;
use super::{PLIST_SSH_WARNING, PLIST_SSL_WARNING};
use crate::config::{DriverKind, Endpoint};
use crate::import::{SourcePassword, SourceTarget};

const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/navicat/conn.plist"
));

fn network(
    driver: DriverKind,
    host: &str,
    port: u16,
    user: &str,
    db: Option<&str>,
) -> SourceTarget {
    SourceTarget::Supported {
        driver,
        endpoint: Endpoint::Network {
            host: host.into(),
            port,
            username: user.into(),
            database: db.map(str::to_owned),
        },
    }
}

#[test]
fn parses_connections_grouped_by_type() {
    let sources = parse_conn_plist(FIXTURE).unwrap();
    let ids: Vec<_> = sources.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "MySQL:本地 MySQL",
            "MySQL:测试 MySQL",
            "PostgreSQL:分析库",
            "REDIS:会话缓存"
        ]
    );
    assert!(
        sources.iter().all(|s| s.password == SourcePassword::Absent),
        "conn.plist 里没有密码"
    );
}

#[test]
fn maps_fields_and_flags_tunnels_and_ssl() {
    let sources = parse_conn_plist(FIXTURE).unwrap();
    assert_eq!(
        sources[0].target,
        network(DriverKind::Mysql, "127.0.0.1", 3306, "root", None)
    );
    assert!(sources[0].warnings.is_empty());

    assert_eq!(
        sources[1].target,
        network(
            DriverKind::Mysql,
            "mysql.example.test",
            13306,
            "tester",
            None
        )
    );
    assert_eq!(sources[1].warnings, [PLIST_SSH_WARNING]);
    assert_eq!(sources[1].ssh, None, "格式没核实过,不猜字段");

    assert_eq!(sources[2].source_type, "PostgreSQL");
    assert_eq!(
        sources[2].target,
        network(
            DriverKind::Postgres,
            "pg.example.test",
            5432,
            "analyst",
            Some("warehouse")
        )
    );
    assert_eq!(sources[2].warnings, [PLIST_SSL_WARNING]);

    assert_eq!(sources[3].source_type, "Redis");
    assert_eq!(
        sources[3].target,
        network(DriverKind::Redis, "redis.example.test", 6379, "", None)
    );
}

#[test]
fn binary_plist_gives_the_same_result() {
    let value = plist::Value::from_reader(std::io::Cursor::new(FIXTURE)).unwrap();
    let mut binary = Vec::new();
    value.to_writer_binary(&mut binary).unwrap();
    assert_eq!(
        parse_conn_plist(&binary).unwrap(),
        parse_conn_plist(FIXTURE).unwrap()
    );
}

#[test]
fn accepts_integer_ports_and_missing_optional_fields() {
    let mut fields = plist::Dictionary::new();
    fields.insert("host".into(), "db.example.test".into());
    fields.insert("port".into(), plist::Value::Integer(3310.into()));
    fields.insert("username".into(), "u".into());
    let conns = plist::Dictionary::from_iter([("库".to_owned(), plist::Value::Dictionary(fields))]);
    let types =
        plist::Dictionary::from_iter([("MariaDB".to_owned(), plist::Value::Dictionary(conns))]);
    let project = plist::Dictionary::from_iter([("0".to_owned(), plist::Value::Dictionary(types))]);
    let root = plist::Dictionary::from_iter([("0".to_owned(), plist::Value::Dictionary(project))]);
    let mut xml = Vec::new();
    plist::Value::Dictionary(root)
        .to_writer_xml(&mut xml)
        .unwrap();

    let sources = parse_conn_plist(&xml).unwrap();
    assert_eq!(
        sources[0].target,
        network(DriverKind::Mariadb, "db.example.test", 3310, "u", None)
    );
}

#[test]
fn rejects_files_that_are_not_plists() {
    let error = parse_conn_plist(b"{ not a plist").unwrap_err();
    assert_eq!(error.code(), "unsupported");
}

#[test]
fn empty_or_unrecognized_groups_mean_no_groups() {
    assert!(parse_vgroups("{}").is_empty());
    assert!(parse_vgroups("").is_empty());
    assert!(parse_vgroups("not json").is_empty());
    assert!(parse_vgroups(r#"{"version": 1, "items": [1, 2]}"#).is_empty());
}

#[test]
fn groups_map_connection_names_to_their_innermost_group() {
    let json = r#"{
        "groups": [
            {"name": "生产", "children": [
                {"type": "connection", "name": "订单库"},
                {"name": "报表", "items": ["报表 PG", {"connectionName": "分析库"}]}
            ]},
            {"name": "空组", "children": []}
        ]
    }"#;
    let groups = parse_vgroups(json);
    assert_eq!(groups.get("订单库").map(String::as_str), Some("生产"));
    assert_eq!(groups.get("报表 PG").map(String::as_str), Some("报表"));
    assert_eq!(groups.get("分析库").map(String::as_str), Some("报表"));
    assert_eq!(groups.len(), 3);
}
