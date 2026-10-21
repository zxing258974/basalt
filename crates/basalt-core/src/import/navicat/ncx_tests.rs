use super::*;
use crate::config::{DriverKind, Endpoint};
use crate::import::navicat::TUNNEL_SSL_WARNING;
use crate::import::{SourcePassword, SourceTarget};

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/navicat/export.ncx"
));
/// 夹具里的假密码明文。
const FAKE_PASSWORDS: [&str; 3] = ["Fake-Pass#2026", "测试密码", "sixteen-chars-pw"];

fn network(host: &str, port: u16, username: &str, database: Option<&str>) -> SourceTarget {
    let driver = if port == 3307 {
        DriverKind::Mariadb
    } else if port == 5433 {
        DriverKind::Postgres
    } else {
        DriverKind::Mysql
    };
    SourceTarget::Supported {
        driver,
        endpoint: Endpoint::Network {
            host: host.into(),
            port,
            username: username.into(),
            database: database.map(str::to_owned),
        },
    }
}

#[test]
fn parses_every_connection_in_the_export() {
    let sources = parse_ncx(FIXTURE).unwrap();
    let ids: Vec<_> = sources.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "MYSQL:订单库",
            "POSTGRESQL:报表 PG",
            "MARIADB:跳板机后的库",
            "SQLITE:本地笔记",
            "MYSQL:旧版导出",
            "REDIS:缓存",
            "SQLSERVER:财务",
            "SQLITE:无路径的 SQLite",
            "MYSQL:订单库#2",
        ]
    );
}

#[test]
fn maps_network_connections_and_decrypts_their_passwords() {
    let sources = parse_ncx(FIXTURE).unwrap();

    assert_eq!(sources[0].name, "订单库");
    assert_eq!(sources[0].source_type, "MySQL");
    assert_eq!(
        sources[0].target,
        network("mysql.example.test", 3306, "app_user", None)
    );
    assert_eq!(
        sources[0].password,
        SourcePassword::Plain("Fake-Pass#2026".into())
    );
    assert!(sources[0].warnings.is_empty());

    assert_eq!(sources[1].source_type, "PostgreSQL");
    assert_eq!(
        sources[1].target,
        network("192.0.2.10", 5433, "report_ro", Some("reports"))
    );
    assert_eq!(
        sources[1].password,
        SourcePassword::Plain("测试密码".into())
    );
    assert_eq!(sources[1].warnings, [TUNNEL_SSL_WARNING], "用了 SSL");

    assert_eq!(sources[2].target, network("10.0.0.8", 3307, "ops", None));
    assert_eq!(sources[2].password, SourcePassword::Absent, "没勾保存密码");
    assert_eq!(sources[2].warnings, [TUNNEL_SSL_WARNING], "用了 SSH 隧道");

    assert_eq!(
        sources[8].warnings,
        [TUNNEL_SSL_WARNING],
        "HTTP 隧道也算隧道"
    );
}

#[test]
fn sqlite_is_imported_only_with_a_file_path() {
    let sources = parse_ncx(FIXTURE).unwrap();
    if cfg!(windows) {
        // 夹具是 macOS 上导出的:/Users/... 在 Windows 上没有盘符,不算绝对路径。
        assert!(
            matches!(&sources[3].target, SourceTarget::Unsupported(reason) if reason.contains("绝对路径"))
        );
    } else {
        assert_eq!(
            sources[3].target,
            SourceTarget::Supported {
                driver: DriverKind::Sqlite,
                endpoint: Endpoint::File {
                    path: "/Users/example/notes.db".into()
                },
            }
        );
    }
    assert!(
        matches!(&sources[7].target, SourceTarget::Unsupported(reason) if reason.contains("路径"))
    );
}

#[test]
fn undecryptable_password_does_not_abort_the_import() {
    let sources = parse_ncx(FIXTURE).unwrap();
    assert_eq!(sources[4].password, SourcePassword::Undecryptable);
    assert!(matches!(sources[4].target, SourceTarget::Supported { .. }));
}

#[test]
fn unsupported_types_explain_why() {
    let sources = parse_ncx(FIXTURE).unwrap();
    for (index, label) in [(5, "Redis"), (6, "SQL Server")] {
        assert_eq!(sources[index].source_type, label);
        let SourceTarget::Unsupported(reason) = &sources[index].target else {
            panic!("{label} 应该是不支持");
        };
        assert!(reason.contains(label), "{reason}");
        assert!(
            reason.contains("MySQL、MariaDB、PostgreSQL 和 SQLite"),
            "{reason}"
        );
    }
}

#[test]
fn parsed_connections_never_print_plaintext() {
    let debug = format!("{:?}", parse_ncx(FIXTURE).unwrap());
    for password in FAKE_PASSWORDS {
        assert!(!debug.contains(password), "{debug}");
    }
}

#[test]
fn unescapes_attributes_and_tolerates_bom_and_empty_exports() {
    let xml = "\u{feff}<?xml version=\"1.0\"?><Connections><Connection ConnectionName=\"A&amp;B &lt;测试&gt;\" ConnType=\"MySQL\" Host=\"h\" Port=\"\" UserName=\"u\"/></Connections>";
    let sources = parse_ncx(xml).unwrap();
    assert_eq!(sources[0].name, "A&B <测试>");
    assert_eq!(
        sources[0].target,
        network("h", 3306, "u", None),
        "空端口用默认端口"
    );

    assert!(
        parse_ncx("<Connections Ver=\"1.5\"></Connections>")
            .unwrap()
            .is_empty()
    );
    assert!(parse_ncx("<Connections/>").unwrap().is_empty());
}

#[test]
fn rejects_files_that_are_not_navicat_exports() {
    let error = parse_ncx("<plist><dict/></plist>").unwrap_err();
    assert_eq!(error.code(), "unsupported");
    assert!(error.to_string().contains(".ncx"), "{error}");

    assert_eq!(parse_ncx("").unwrap_err().code(), "unsupported");
}

#[test]
fn malformed_xml_error_does_not_echo_the_ciphertext() {
    let xml = "<Connections><Connection ConnectionName=\"x\" Password=\"7CD6F821FD5487C86ADFB4EE5FA0C277\" <broken";
    let error = parse_ncx(xml).unwrap_err();
    assert_eq!(error.code(), "unsupported");
    let message = error.to_string();
    assert!(!message.contains("7CD6F821"), "{message}");
}
