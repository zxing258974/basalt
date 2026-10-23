use super::*;

const FAKE_PASSWORD: &str = "fake-secret-42";

fn network(host: &str, port: u16, username: &str) -> Endpoint {
    Endpoint::Network {
        host: host.into(),
        port,
        username: username.into(),
        database: None,
    }
}

fn source(id: &str, name: &str, driver: DriverKind, endpoint: Endpoint) -> SourceConnection {
    SourceConnection {
        id: id.into(),
        name: name.into(),
        group: None,
        source_type: "MySQL".into(),
        target: SourceTarget::Supported { driver, endpoint },
        password: SourcePassword::Plain(FAKE_PASSWORD.into()),
        ssh: None,
        ssh_secret: SourcePassword::Absent,
        ssl: SslSettings::default(),
        warnings: vec![],
    }
}

fn saved(name: &str, driver: DriverKind, endpoint: Endpoint) -> ConnectionConfig {
    ConnectionConfig {
        id: format!("saved-{name}"),
        name: name.into(),
        group: None,
        driver,
        endpoint,
        save_password: false,
        ssh: None,
        ssl: SslSettings::default(),
    }
}

/// 已经在密码存储里存了密码的已有连接。
fn with_saved_password(config: ConnectionConfig) -> ConnectionConfig {
    ConnectionConfig {
        save_password: true,
        ..config
    }
}

fn without_password(source: SourceConnection) -> SourceConnection {
    SourceConnection {
        password: SourcePassword::Absent,
        ..source
    }
}

fn list(configs: Vec<ConnectionConfig>) -> ConnectionList {
    configs
        .into_iter()
        .fold(ConnectionList::default(), |list, c| list.upserted(c))
}

#[test]
fn unmatched_supported_connection_is_new() {
    let sources = [source(
        "a",
        "订单库",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    )];
    let candidates = classify(&sources, &ConnectionList::default());
    assert_eq!(candidates[0].status, CandidateStatus::New);
    assert_eq!(candidates[0].driver, Some(DriverKind::Mysql));
    assert_eq!(candidates[0].password, PasswordStatus::Included);
}

#[test]
fn same_driver_host_port_and_user_counts_as_existing() {
    let existing = list(vec![with_saved_password(saved(
        "别名",
        DriverKind::Mysql,
        network("DB.Test", 3306, "app"),
    ))]);
    let sources = [source(
        "a",
        "订单库",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    )];
    assert_eq!(
        classify(&sources, &existing)[0].status,
        CandidateStatus::Existing {
            existing_name: "别名".into()
        }
    );
}

#[test]
fn different_port_user_or_driver_is_not_a_duplicate() {
    let existing = list(vec![saved(
        "别名",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    )]);
    let sources = [
        source(
            "p",
            "端口不同",
            DriverKind::Mysql,
            network("db.test", 3307, "app"),
        ),
        source(
            "u",
            "用户不同",
            DriverKind::Mysql,
            network("db.test", 3306, "App"),
        ),
        source(
            "d",
            "类型不同",
            DriverKind::Mariadb,
            network("db.test", 3306, "app"),
        ),
    ];
    let statuses: Vec<_> = classify(&sources, &existing)
        .into_iter()
        .map(|c| c.status)
        .collect();
    assert_eq!(statuses, vec![CandidateStatus::New; 3]);
}

#[test]
fn same_name_counts_as_existing_even_with_another_address() {
    let existing = list(vec![saved(
        "订单库",
        DriverKind::Postgres,
        network("pg.test", 5432, "x"),
    )]);
    let sources = [source(
        "a",
        "订单库",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    )];
    assert!(matches!(
        classify(&sources, &existing)[0].status,
        CandidateStatus::Existing { .. }
    ));
}

#[test]
fn later_duplicates_within_the_same_source_are_not_checked_by_default() {
    let sources = [
        source(
            "1",
            "订单库",
            DriverKind::Mysql,
            network("a.test", 3306, "app"),
        ),
        source(
            "2",
            "订单库",
            DriverKind::Mysql,
            network("b.test", 3306, "app"),
        ),
        source(
            "3",
            "别名",
            DriverKind::Mysql,
            network("A.test", 3306, "app"),
        ),
        source(
            "4",
            "另一个",
            DriverKind::Mysql,
            network("c.test", 3306, "app"),
        ),
    ];
    let statuses: Vec<_> = classify(&sources, &ConnectionList::default())
        .into_iter()
        .map(|c| c.status)
        .collect();
    let existing = |name: &str| CandidateStatus::Existing {
        existing_name: name.into(),
    };
    assert_eq!(
        statuses,
        [
            CandidateStatus::New,
            existing("订单库"),
            existing("订单库"),
            CandidateStatus::New
        ]
    );
}

#[test]
fn sqlite_duplicates_are_matched_by_file_path() {
    let file = |path: &str| Endpoint::File { path: path.into() };
    let existing = list(vec![saved("笔记", DriverKind::Sqlite, file("/data/a.db"))]);
    let sources = [
        source("1", "同一个文件", DriverKind::Sqlite, file("/data/a.db")),
        source("2", "另一个文件", DriverKind::Sqlite, file("/data/b.db")),
    ];
    let candidates = classify(&sources, &existing);
    assert!(matches!(
        candidates[0].status,
        CandidateStatus::Existing { .. }
    ));
    assert_eq!(candidates[1].status, CandidateStatus::New);
}

#[test]
fn unsupported_source_keeps_its_reason_and_has_no_target() {
    let sources = [SourceConnection {
        target: SourceTarget::Unsupported("Basalt 暂不支持 MongoDB".into()),
        source_type: "MongoDB".into(),
        password: SourcePassword::Absent,
        ..source("r", "缓存", DriverKind::Mysql, network("h", 1, "u"))
    }];
    let candidate = &classify(&sources, &ConnectionList::default())[0];
    assert_eq!(
        candidate.status,
        CandidateStatus::Unsupported {
            reason: "Basalt 暂不支持 MongoDB".into()
        }
    );
    assert_eq!(candidate.driver, None);
    assert_eq!(candidate.endpoint, None);
    assert_eq!(candidate.password, PasswordStatus::Absent);
}

#[test]
fn draft_saves_password_only_when_the_source_brings_one() {
    let with_password = source("a", "库", DriverKind::Mysql, network("h", 3306, "u"));
    let draft = with_password.to_draft().unwrap();
    assert!(draft.save_password);
    assert_eq!(draft.password.as_deref(), Some(FAKE_PASSWORD));
    assert_eq!(draft.id, None);

    for password in [SourcePassword::Absent, SourcePassword::Undecryptable] {
        let draft = SourceConnection {
            password,
            ..with_password.clone()
        }
        .to_draft()
        .unwrap();
        assert!(
            !draft.save_password,
            "没有密码就不标保存,打开前先弹密码框,不会拿空密码先失败一次"
        );
        assert_eq!(draft.password, None);
    }

    let unsupported = SourceConnection {
        target: SourceTarget::Unsupported("不支持".into()),
        ..with_password
    };
    assert!(unsupported.to_draft().is_none());
}

#[test]
fn plaintext_never_shows_up_in_debug_or_serialized_candidates() {
    let sources = [source(
        "a",
        "库",
        DriverKind::Mysql,
        network("h", 3306, "u"),
    )];
    let debug = format!("{sources:?}");
    assert!(!debug.contains(FAKE_PASSWORD), "{debug}");

    let json = serde_json::to_string(&classify(&sources, &ConnectionList::default())).unwrap();
    assert!(!json.contains(FAKE_PASSWORD), "{json}");
    assert!(json.contains(r#""password":"included""#), "{json}");
    assert!(json.contains(r#""status":{"kind":"new"}"#), "{json}");
}

#[test]
fn existing_status_serializes_with_camel_case_fields() {
    let status = CandidateStatus::Existing {
        existing_name: "旧".into(),
    };
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"kind":"existing","existingName":"旧"}"#
    );
}

#[test]
fn source_password_fills_a_matching_connection_that_has_none() {
    let existing = list(vec![saved(
        "别名",
        DriverKind::Mysql,
        network("DB.Test", 3306, "app"),
    )]);
    let sources = [source(
        "a",
        "订单库",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    )];
    assert_eq!(
        classify(&sources, &existing)[0].status,
        CandidateStatus::FillPassword {
            existing_id: "saved-别名".into(),
            existing_name: "别名".into(),
        }
    );
}

#[test]
fn password_is_never_filled_into_a_connection_matched_only_by_name() {
    let existing = list(vec![saved(
        "订单库",
        DriverKind::Mysql,
        network("other.test", 3306, "app"),
    )]);
    let sources = [source(
        "a",
        "订单库",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    )];
    assert!(matches!(
        classify(&sources, &existing)[0].status,
        CandidateStatus::Existing { .. }
    ));
}

#[test]
fn source_without_password_cannot_fill_anything() {
    let existing = list(vec![saved(
        "别名",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    )]);
    let sources = [without_password(source(
        "a",
        "订单库",
        DriverKind::Mysql,
        network("db.test", 3306, "app"),
    ))];
    assert!(matches!(
        classify(&sources, &existing)[0].status,
        CandidateStatus::Existing { .. }
    ));
}

#[test]
fn fill_password_status_serializes_with_camel_case_fields() {
    let status = CandidateStatus::FillPassword {
        existing_id: "c1".into(),
        existing_name: "旧".into(),
    };
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"kind":"fillPassword","existingId":"c1","existingName":"旧"}"#
    );
}

fn tunnel(host: &str) -> SshTunnel {
    SshTunnel {
        host: host.into(),
        port: 22,
        username: "jump".into(),
        auth: crate::config::SshAuthMethod::Password,
        private_key_path: None,
        save_secret: false,
        jump_hosts: None,
    }
}

#[test]
fn tunneled_connections_are_told_apart_by_their_ssh_server() {
    // 隧道后面的库地址常常都是 127.0.0.1:3306,要连同 SSH 服务器一起比。
    let existing = list(vec![ConnectionConfig {
        ssh: Some(tunnel("bastion-a")),
        ..saved(
            "A 机房",
            DriverKind::Mysql,
            network("127.0.0.1", 3306, "app"),
        )
    }]);
    let behind = |id: &str, name: &str, host: Option<&str>| SourceConnection {
        ssh: host.map(tunnel),
        ..source(
            id,
            name,
            DriverKind::Mysql,
            network("127.0.0.1", 3306, "app"),
        )
    };
    let sources = [
        behind("same", "别名", Some("BASTION-A")),
        behind("other", "B 机房", Some("bastion-b")),
        behind("direct", "本机", None),
    ];
    let candidates = classify(&sources, &existing);
    assert_eq!(
        candidates[0].status,
        CandidateStatus::FillPassword {
            existing_id: "saved-A 机房".into(),
            existing_name: "A 机房".into()
        }
    );
    assert_eq!(candidates[1].status, CandidateStatus::New);
    assert_eq!(candidates[2].status, CandidateStatus::New);
}

#[test]
fn candidates_and_drafts_carry_tunnel_and_ssl() {
    let with_tunnel = SourceConnection {
        ssh: Some(tunnel("bastion")),
        ssh_secret: SourcePassword::Plain("ssh-fake".into()),
        ssl: SslSettings {
            mode: SslMode::Required,
            ..SslSettings::default()
        },
        ..source(
            "t",
            "隧道库",
            DriverKind::Postgres,
            network("10.0.0.5", 5432, "u"),
        )
    };
    let candidate = &classify(
        std::slice::from_ref(&with_tunnel),
        &ConnectionList::default(),
    )[0];
    assert_eq!(
        candidate.ssh.as_ref().map(|t| t.host.as_str()),
        Some("bastion")
    );
    assert_eq!(candidate.ssh_secret, Some(PasswordStatus::Included));
    assert_eq!(candidate.ssl_mode, SslMode::Required);
    let json = serde_json::to_string(candidate).unwrap();
    assert!(!json.contains("ssh-fake"), "{json}");

    let draft = with_tunnel.to_draft().unwrap();
    assert_eq!(draft.ssh_secret.as_deref(), Some("ssh-fake"));
    assert!(draft.ssh.as_ref().unwrap().save_secret);
    assert_eq!(draft.ssl.mode, SslMode::Required);

    let without_secret = SourceConnection {
        ssh_secret: SourcePassword::Undecryptable,
        ..with_tunnel
    }
    .to_draft()
    .unwrap();
    assert_eq!(without_secret.ssh_secret, None);
    assert!(
        !without_secret.ssh.unwrap().save_secret,
        "没带上 SSH 密码就不标保存"
    );

    let direct = &classify(
        &[source(
            "d",
            "直连",
            DriverKind::Mysql,
            network("h", 3306, "u"),
        )],
        &ConnectionList::default(),
    )[0];
    assert_eq!(direct.ssh, None);
    assert_eq!(direct.ssh_secret, None);
}
