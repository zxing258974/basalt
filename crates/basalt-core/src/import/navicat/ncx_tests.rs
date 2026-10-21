use super::*;
use crate::config::{DriverKind, Endpoint, SshAuthMethod, SshTunnel, SslMode};
use crate::import::navicat::HTTP_TUNNEL_WARNING;
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
            "POSTGRESQL:密钥跳板后的 PG",
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
    assert_eq!(
        sources[1].ssl.mode,
        SslMode::VerifyIdentity,
        "PG 的 VERIFY-FULL"
    );
    assert!(sources[1].warnings.is_empty(), "{:?}", sources[1].warnings);

    assert_eq!(sources[2].target, network("10.0.0.8", 3307, "ops", None));
    assert_eq!(sources[2].password, SourcePassword::Absent, "没勾保存密码");
    assert_eq!(
        sources[2].ssh,
        Some(SshTunnel {
            host: "bastion.example.test".into(),
            port: 22,
            username: "jump".into(),
            auth: SshAuthMethod::Password,
            private_key_path: None,
            save_secret: true,
        })
    );
    assert_eq!(
        sources[2].ssh_secret,
        SourcePassword::Plain("sixteen-chars-pw".into())
    );
    assert!(sources[2].warnings.is_empty(), "{:?}", sources[2].warnings);
    assert_eq!(
        sources[2].ssl.mode,
        SslMode::Preferred,
        "没开 SSL 时沿用默认"
    );

    assert_eq!(sources[8].warnings, [HTTP_TUNNEL_WARNING]);
}

#[test]
fn key_login_and_ssl_files_from_another_computer_are_flagged() {
    let sources = parse_ncx(FIXTURE).unwrap();
    let source = &sources[9];
    let ssh = source.ssh.clone().expect("隧道照样迁移");
    assert_eq!((ssh.host.as_str(), ssh.port), ("jump.example.test", 2222));
    assert_eq!(ssh.username, "deploy");
    assert_eq!(source.ssl.mode, SslMode::VerifyCa, "经隧道不能核对主机名");
    let has = |needle: &str| source.warnings.iter().any(|w| w.contains(needle));
    assert!(has("改成「校验 CA」"), "{:?}", source.warnings);
    assert!(has("AES256-SHA"), "{:?}", source.warnings);
    if cfg!(windows) {
        // 夹具是 Windows 上导出的,路径在 Windows 上能用。
        assert_eq!(ssh.auth, SshAuthMethod::PrivateKey);
        assert_eq!(
            source.ssh_secret,
            SourcePassword::Plain("Fake-Passphrase!".into())
        );
        assert!(source.ssl.client_cert_path.is_some());
    } else {
        assert_eq!(
            ssh.auth,
            SshAuthMethod::Password,
            "私钥路径用不了,先改成密码登录"
        );
        assert_eq!(source.ssh_secret, SourcePassword::Absent);
        assert!(
            has("id_ed25519 在这台电脑上用不了"),
            "{:?}",
            source.warnings
        );
        assert!(has("客户端证书"), "{:?}", source.warnings);
        assert_eq!(source.ssl.client_cert_path, None);
        assert_eq!(source.ssl.client_key_path, None);
    }
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
