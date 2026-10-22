use super::*;

/// 这台电脑上算绝对路径的写法。
fn here(name: &str) -> String {
    std::env::temp_dir().join(name).display().to_string()
}

fn ssh(auth_method: &str) -> RawSsh {
    RawSsh {
        host: " bastion.example.test ".into(),
        port: "".into(),
        username: "jump".into(),
        auth_method: auth_method.into(),
        ..RawSsh::default()
    }
}

#[test]
fn password_tunnel_keeps_its_saved_password() {
    let raw = RawSsh {
        password: SourcePassword::Plain("pw".into()),
        ..ssh("PASSWORD")
    };
    let mapped = map_transport(DriverKind::Mysql, Some(&raw), None);
    let tunnel = mapped.ssh.unwrap();
    assert_eq!(tunnel.host, "bastion.example.test");
    assert_eq!(tunnel.port, DEFAULT_SSH_PORT, "空端口用 22");
    assert_eq!(tunnel.auth, SshAuthMethod::Password);
    assert!(tunnel.save_secret);
    assert_eq!(mapped.ssh_secret, SourcePassword::Plain("pw".into()));
    assert!(mapped.warnings.is_empty());
    assert_eq!(mapped.ssl, SslSettings::default(), "没开 SSL 时沿用默认");
}

#[test]
fn key_tunnel_carries_path_and_passphrase() {
    let raw = RawSsh {
        port: "2222".into(),
        private_key: here("id_ed25519"),
        passphrase: SourcePassword::Plain("kp".into()),
        ..ssh("PUBLICKEY")
    };
    let mapped = map_transport(DriverKind::Postgres, Some(&raw), None);
    let tunnel = mapped.ssh.unwrap();
    assert_eq!(tunnel.port, 2222);
    assert_eq!(tunnel.auth, SshAuthMethod::PrivateKey);
    assert_eq!(tunnel.private_key_path, Some(here("id_ed25519")));
    assert_eq!(mapped.ssh_secret, SourcePassword::Plain("kp".into()));
}

#[test]
fn key_without_usable_path_falls_back_to_password_with_a_warning() {
    for key in ["", "id_rsa"] {
        let raw = RawSsh {
            private_key: key.into(),
            passphrase: SourcePassword::Plain("kp".into()),
            ..ssh("PUBLICKEY")
        };
        let mapped = map_transport(DriverKind::Mysql, Some(&raw), None);
        let tunnel = mapped.ssh.unwrap();
        assert_eq!(tunnel.auth, SshAuthMethod::Password, "{key:?}");
        assert!(!tunnel.save_secret, "口令不能当密码用");
        assert_eq!(mapped.ssh_secret, SourcePassword::Absent);
        assert_eq!(mapped.warnings.len(), 1, "{:?}", mapped.warnings);
        assert!(
            mapped.warnings[0].contains("「SSH」页"),
            "{:?}",
            mapped.warnings
        );
    }
}

#[test]
fn incomplete_tunnel_is_dropped_and_bad_port_defaults() {
    let missing_user = RawSsh {
        username: " ".into(),
        ..ssh("PASSWORD")
    };
    let mapped = map_transport(DriverKind::Mysql, Some(&missing_user), None);
    assert_eq!(mapped.ssh, None);
    assert!(mapped.warnings[0].contains("没有迁移"));

    let bad_port = RawSsh {
        port: "0".into(),
        password: SourcePassword::Undecryptable,
        ..ssh("PASSWORD")
    };
    let mapped = map_transport(DriverKind::Mysql, Some(&bad_port), None);
    assert_eq!(mapped.ssh.unwrap().port, DEFAULT_SSH_PORT);
    assert_eq!(mapped.warnings.len(), 2, "{:?}", mapped.warnings);
    assert!(mapped.warnings[1].contains("解不开"));
}

#[test]
fn postgres_ssl_modes_follow_navicat_names() {
    for (raw, expected) in [
        ("DISABLE", SslMode::Disabled),
        ("allow", SslMode::Preferred),
        ("PREFER", SslMode::Preferred),
        ("", SslMode::Required),
        ("REQUIRE", SslMode::Required),
        ("VERIFY-CA", SslMode::VerifyCa),
        ("VERIFY-FULL", SslMode::VerifyIdentity),
    ] {
        let ssl = RawSsl {
            pg_mode: raw.into(),
            ..RawSsl::default()
        };
        let mapped = map_transport(DriverKind::Postgres, None, Some(&ssl));
        assert_eq!(mapped.ssl.mode, expected, "{raw}");
        assert!(mapped.warnings.is_empty());
    }
    let odd = RawSsl {
        pg_mode: "SOMETIMES".into(),
        ..RawSsl::default()
    };
    let mapped = map_transport(DriverKind::Postgres, None, Some(&odd));
    assert_eq!(mapped.ssl.mode, SslMode::Required);
    assert!(mapped.warnings[0].contains("SOMETIMES"));
}

#[test]
fn mysql_ssl_is_required_or_verifies_the_ca() {
    let on = RawSsl {
        pg_mode: "VERIFY-FULL".into(),
        ..RawSsl::default()
    };
    assert_eq!(
        map_transport(DriverKind::Mysql, None, Some(&on)).ssl.mode,
        SslMode::Required,
        "MySQL 不看 PG 的模式"
    );
    let verify = RawSsl {
        verify_ca: true,
        ca_cert: here("ca.pem"),
        ..RawSsl::default()
    };
    let mapped = map_transport(DriverKind::Mariadb, None, Some(&verify));
    assert_eq!(mapped.ssl.mode, SslMode::VerifyCa);
    assert_eq!(mapped.ssl.ca_path, Some(here("ca.pem")));
}

#[test]
fn hostname_check_is_relaxed_when_allowed_or_tunneled() {
    let full = RawSsl {
        pg_mode: "VERIFY-FULL".into(),
        allow_invalid_host_name: Some(true),
        ..RawSsl::default()
    };
    let mapped = map_transport(DriverKind::Postgres, None, Some(&full));
    assert_eq!(mapped.ssl.mode, SslMode::VerifyCa);
    assert!(mapped.warnings.is_empty());

    let tunneled = RawSsl {
        allow_invalid_host_name: Some(false),
        ..full
    };
    let raw_ssh = ssh("PASSWORD");
    let mapped = map_transport(DriverKind::Postgres, Some(&raw_ssh), Some(&tunneled));
    assert_eq!(mapped.ssl.mode, SslMode::VerifyCa);
    assert!(mapped.warnings[0].contains("校验 CA"));
}

#[test]
fn client_certificate_needs_both_files_and_extras_are_reported() {
    let both = RawSsl {
        client_auth: true,
        client_cert: here("client.crt"),
        client_key: here("client.key"),
        ..RawSsl::default()
    };
    let mapped = map_transport(DriverKind::Postgres, None, Some(&both));
    assert_eq!(mapped.ssl.client_cert_path, Some(here("client.crt")));
    assert_eq!(mapped.ssl.client_key_path, Some(here("client.key")));
    assert!(mapped.warnings.is_empty());

    let unused = RawSsl {
        client_auth: false,
        ..both
    };
    let mapped = map_transport(DriverKind::Postgres, None, Some(&unused));
    assert_eq!(
        mapped.ssl.client_cert_path, None,
        "没勾「使用验证」时不用客户端证书"
    );

    let half = RawSsl {
        client_auth: true,
        client_cert: here("client.crt"),
        client_key: "client.key".into(),
        client_key_password: true,
        cipher: "AES256-SHA".into(),
        crl: here("crl.pem"),
        ca_cert: "relative-ca.pem".into(),
        ..RawSsl::default()
    };
    let mapped = map_transport(DriverKind::Postgres, None, Some(&half));
    assert_eq!(mapped.ssl.client_cert_path, None);
    assert_eq!(mapped.ssl.ca_path, None);
    let joined = mapped.warnings.join("\n");
    for needle in [
        "relative-ca.pem",
        "client.key",
        "只迁移了一个",
        "口令",
        "AES256-SHA",
        "吊销列表",
    ] {
        assert!(joined.contains(needle), "缺「{needle}」：{joined}");
    }
}

#[test]
fn mysql_keeps_the_host_name_check_navicat_had() {
    let strict = RawSsl {
        verify_ca: true,
        allow_invalid_host_name: Some(false),
        ..RawSsl::default()
    };
    let mapped = map_transport(DriverKind::Mysql, None, Some(&strict));
    assert_eq!(mapped.ssl.mode, SslMode::VerifyIdentity);
    assert!(mapped.warnings.is_empty());

    // 经隧道时核对不了主机名:照旧降成校验 CA 并提醒。
    let raw_ssh = ssh("PASSWORD");
    let tunneled = map_transport(DriverKind::Mysql, Some(&raw_ssh), Some(&strict));
    assert_eq!(tunneled.ssl.mode, SslMode::VerifyCa);
    assert!(tunneled.warnings.iter().any(|w| w.contains("校验 CA")));

    // 允许无效主机名、或者老文件里没有这一项:只校验 CA。
    for allow in [Some(true), None] {
        let relaxed = RawSsl {
            allow_invalid_host_name: allow,
            ..strict.clone()
        };
        let mapped = map_transport(DriverKind::Mariadb, None, Some(&relaxed));
        assert_eq!(mapped.ssl.mode, SslMode::VerifyCa, "{allow:?}");
    }
}
