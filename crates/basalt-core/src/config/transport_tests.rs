use super::*;
use crate::config::{ConnectionDraft, DriverKind, Endpoint, validate};

fn abs(name: &str) -> String {
    std::env::temp_dir().join(name).display().to_string()
}

fn tunnel(auth: SshAuthMethod, key: Option<&str>) -> SshTunnel {
    SshTunnel {
        host: " jump.example.com ".into(),
        port: 22,
        username: " deploy ".into(),
        auth,
        private_key_path: key.map(str::to_owned),
        save_secret: true,
    }
}

fn draft(ssh: Option<SshTunnel>, ssl: SslSettings) -> ConnectionDraft {
    ConnectionDraft {
        id: None,
        name: "远程".into(),
        group: None,
        driver: DriverKind::Postgres,
        endpoint: Endpoint::Network {
            host: "10.0.0.5".into(),
            port: 5432,
            username: "app".into(),
            database: None,
        },
        save_password: true,
        password: None,
        ssh,
        ssl,
        ssh_secret: None,
    }
}

fn fields(draft: &ConnectionDraft) -> Vec<String> {
    validate(draft).into_iter().map(|e| e.field).collect()
}

#[test]
fn password_tunnel_needs_host_port_and_user() {
    let ok = draft(
        Some(tunnel(SshAuthMethod::Password, None)),
        SslSettings::default(),
    );
    assert!(fields(&ok).is_empty());

    let blank = SshTunnel {
        host: " ".into(),
        port: 0,
        username: "".into(),
        ..tunnel(SshAuthMethod::Password, None)
    };
    assert_eq!(
        fields(&draft(Some(blank), SslSettings::default())),
        ["sshHost", "sshPort", "sshUsername"]
    );

    let spaced = SshTunnel {
        host: "jump host".into(),
        ..tunnel(SshAuthMethod::Password, None)
    };
    assert_eq!(
        fields(&draft(Some(spaced), SslSettings::default())),
        ["sshHost"]
    );
}

#[test]
fn private_key_tunnel_needs_absolute_key_path() {
    let path = abs("id_ed25519");
    let ok = draft(
        Some(tunnel(SshAuthMethod::PrivateKey, Some(&path))),
        SslSettings::default(),
    );
    assert!(fields(&ok).is_empty());
    for key in [None, Some(""), Some("id_ed25519")] {
        let bad = draft(
            Some(tunnel(SshAuthMethod::PrivateKey, key)),
            SslSettings::default(),
        );
        assert_eq!(fields(&bad), ["sshPrivateKey"], "{key:?}");
    }
}

#[test]
fn ssl_paths_must_be_absolute_and_cert_key_come_together() {
    let ok = SslSettings {
        mode: SslMode::VerifyCa,
        ca_path: Some(abs("ca.pem")),
        client_cert_path: Some(abs("client.pem")),
        client_key_path: Some(abs("client.key")),
    };
    assert!(fields(&draft(None, ok.clone())).is_empty());

    let relative = SslSettings {
        ca_path: Some("ca.pem".into()),
        ..ok.clone()
    };
    assert_eq!(fields(&draft(None, relative)), ["sslCa"]);

    let cert_only = SslSettings {
        client_key_path: Some("  ".into()),
        ..ok.clone()
    };
    assert_eq!(fields(&draft(None, cert_only)), ["sslKey"]);
    let key_only = SslSettings {
        client_cert_path: None,
        ..ok.clone()
    };
    assert_eq!(fields(&draft(None, key_only)), ["sslCert"]);

    // 禁用 SSL 时不管留着的路径。
    let disabled = SslSettings {
        mode: SslMode::Disabled,
        ca_path: Some("relative.pem".into()),
        client_cert_path: Some("c".into()),
        client_key_path: None,
    };
    assert!(fields(&draft(None, disabled)).is_empty());
}

#[test]
fn hostname_check_is_rejected_through_a_tunnel() {
    let verify = SslSettings {
        mode: SslMode::VerifyIdentity,
        ..SslSettings::default()
    };
    assert!(fields(&draft(None, verify.clone())).is_empty());
    assert_eq!(
        fields(&draft(Some(tunnel(SshAuthMethod::Password, None)), verify)),
        ["sslMode"]
    );
    let verify_ca = SslSettings {
        mode: SslMode::VerifyCa,
        ..SslSettings::default()
    };
    assert!(
        fields(&draft(
            Some(tunnel(SshAuthMethod::Password, None)),
            verify_ca
        ))
        .is_empty()
    );
}

#[test]
fn into_config_trims_tunnel_and_drops_unused_paths() {
    let mut ssh = tunnel(SshAuthMethod::Password, Some("/leftover/key"));
    ssh.save_secret = false;
    let ssl = SslSettings {
        mode: SslMode::Required,
        ca_path: Some("  ".into()),
        client_cert_path: None,
        client_key_path: None,
    };
    let config = draft(Some(ssh), ssl).into_config("c".into()).unwrap();
    let ssh = config.ssh.unwrap();
    assert_eq!(ssh.host, "jump.example.com");
    assert_eq!(ssh.username, "deploy");
    assert_eq!(ssh.private_key_path, None, "密码登录不留私钥路径");
    assert!(ssh.needs_prompt());
    assert_eq!(config.ssl.ca_path, None);
    assert_eq!(config.ssl.mode, SslMode::Required);
}

#[test]
fn only_unsaved_password_login_needs_a_prompt() {
    let mut password = tunnel(SshAuthMethod::Password, None);
    assert!(!password.needs_prompt());
    password.save_secret = false;
    assert!(password.needs_prompt());
    let mut key = tunnel(SshAuthMethod::PrivateKey, Some("/k"));
    key.save_secret = false;
    assert!(!key.needs_prompt(), "私钥没有口令时不用问");
}

#[test]
fn ssl_files_lists_only_used_paths() {
    let ssl = SslSettings {
        mode: SslMode::VerifyCa,
        ca_path: Some("/ca.pem".into()),
        client_cert_path: None,
        client_key_path: None,
    };
    assert_eq!(ssl.files(), [("CA 证书", "/ca.pem")]);
    let disabled = SslSettings {
        mode: SslMode::Disabled,
        ..ssl
    };
    assert!(disabled.files().is_empty());
    assert!(SslMode::VerifyCa.verifies_certificate());
    assert!(!SslMode::Required.verifies_certificate());
}

#[test]
fn serializes_with_camel_case_names() {
    let json = serde_json::to_value(tunnel(SshAuthMethod::PrivateKey, Some("/k"))).unwrap();
    assert_eq!(json["auth"], "privateKey");
    assert_eq!(json["privateKeyPath"], "/k");
    assert_eq!(json["saveSecret"], true);
    let ssl = serde_json::to_value(SslSettings {
        mode: SslMode::VerifyIdentity,
        ..SslSettings::default()
    })
    .unwrap();
    assert_eq!(ssl["mode"], "verifyIdentity");
    assert_eq!(ssl["caPath"], serde_json::Value::Null);
}

#[test]
fn ssh_host_names_cannot_carry_known_hosts_syntax() {
    for bad in [
        "jump,prod.example.com",
        "*.example.com",
        "[::1]",
        "@cert",
        "a|b",
    ] {
        let draft = draft(
            Some(SshTunnel {
                host: bad.into(),
                ..tunnel(SshAuthMethod::Password, None)
            }),
            SslSettings::default(),
        );
        assert_eq!(fields(&draft), ["sshHost"], "{bad}");
    }
    let ipv6 = SshTunnel {
        host: "fe80::1".into(),
        ..tunnel(SshAuthMethod::Password, None)
    };
    assert!(fields(&draft(Some(ipv6), SslSettings::default())).is_empty());
}

#[test]
fn a_leading_tilde_means_the_home_folder() {
    let home = std::env::home_dir().expect("测试机有主目录");
    assert_eq!(
        expand_home("~/.ssh/id_ed25519"),
        home.join(".ssh/id_ed25519").display().to_string()
    );
    assert_eq!(expand_home("~"), home.display().to_string());
    assert_eq!(expand_home("/abs/~/x"), "/abs/~/x");
    assert_eq!(expand_home("~other/x"), "~other/x", "别人的主目录不展开");

    let key = draft(
        Some(tunnel(SshAuthMethod::PrivateKey, Some("~/.ssh/id_ed25519"))),
        SslSettings {
            mode: SslMode::VerifyCa,
            ca_path: Some("~/certs/ca.pem".into()),
            ..SslSettings::default()
        },
    );
    assert!(fields(&key).is_empty(), "~ 开头算绝对路径");
    let config = key.into_config("c".into()).unwrap();
    assert_eq!(
        config.ssh.unwrap().private_key_path,
        Some(home.join(".ssh/id_ed25519").display().to_string())
    );
    assert_eq!(
        config.ssl.ca_path,
        Some(home.join("certs/ca.pem").display().to_string())
    );
}
