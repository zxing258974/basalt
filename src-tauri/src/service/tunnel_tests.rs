//! 隧道相关 service 测试:内存密码存储 + 临时配置目录。真的经 sshd 连库在 service_tests.rs(BASALT_IT)。

use std::sync::Arc;

use basalt_core::config::{
    ConnectionDraft, DriverKind, Endpoint, SshAuthMethod, SshTunnel, SslMode, SslSettings,
};
use basalt_core::secret::{MemorySecretStore, SecretStore, password_key, ssh_secret_key};

use super::*;
use crate::service::{self, CommandError};

struct Harness {
    _dir: tempfile::TempDir,
    secrets: Arc<MemorySecretStore>,
    state: AppState,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let state = AppState::new(dir.path().join("config/connections.json"), secrets.clone());
    Harness {
        _dir: dir,
        secrets,
        state,
    }
}

fn tunnel(save_secret: bool) -> SshTunnel {
    SshTunnel {
        host: "jump.example.test".into(),
        port: 22,
        username: "deploy".into(),
        auth: SshAuthMethod::Password,
        private_key_path: None,
        save_secret,
    }
}

fn draft(ssh: Option<SshTunnel>, ssh_secret: Option<&str>) -> ConnectionDraft {
    ConnectionDraft {
        id: None,
        name: "隧道库".into(),
        group: None,
        driver: DriverKind::Mysql,
        endpoint: Endpoint::Network {
            host: "10.0.0.5".into(),
            port: 3306,
            username: "app".into(),
            database: None,
        },
        save_password: true,
        password: Some("db-pass".into()),
        ssh,
        ssl: SslSettings::default(),
        ssh_secret: ssh_secret.map(str::to_owned),
    }
}

#[tokio::test]
async fn ssh_secret_is_saved_kept_and_removed_with_the_tunnel() {
    let h = harness();
    let saved = service::save_connection(&h.state, draft(Some(tunnel(true)), Some("ssh-pass")))
        .await
        .unwrap();
    let key = ssh_secret_key(&saved.id);
    assert_eq!(h.secrets.get(&key).unwrap().as_deref(), Some("ssh-pass"));
    assert_eq!(
        h.secrets.get(&password_key(&saved.id)).unwrap().as_deref(),
        Some("db-pass")
    );

    // 没动 SSH 密码框:沿用。
    let keep = ConnectionDraft {
        id: Some(saved.id.clone()),
        password: None,
        ..draft(Some(tunnel(true)), None)
    };
    service::save_connection(&h.state, keep).await.unwrap();
    assert_eq!(h.secrets.get(&key).unwrap().as_deref(), Some("ssh-pass"));

    // 不再保存 SSH 密码:删掉。
    let forget = ConnectionDraft {
        id: Some(saved.id.clone()),
        password: None,
        ..draft(Some(tunnel(false)), None)
    };
    service::save_connection(&h.state, forget).await.unwrap();
    assert_eq!(h.secrets.get(&key).unwrap(), None);

    // 去掉隧道:同样不留。
    service::save_connection(
        &h.state,
        ConnectionDraft {
            id: Some(saved.id.clone()),
            password: None,
            ..draft(Some(tunnel(true)), Some("again"))
        },
    )
    .await
    .unwrap();
    service::save_connection(
        &h.state,
        ConnectionDraft {
            id: Some(saved.id.clone()),
            password: None,
            ..draft(None, None)
        },
    )
    .await
    .unwrap();
    assert_eq!(h.secrets.get(&key).unwrap(), None);
    assert_eq!(h.state.find(&saved.id).unwrap().ssh, None);
}

#[tokio::test]
async fn duplicate_and_delete_handle_the_ssh_secret_too() {
    let h = harness();
    let saved = service::save_connection(&h.state, draft(Some(tunnel(true)), Some("ssh-pass")))
        .await
        .unwrap();
    let copy = service::duplicate_connection(&h.state, &saved.id)
        .await
        .unwrap();
    assert_eq!(
        h.secrets.get(&ssh_secret_key(&copy.id)).unwrap().as_deref(),
        Some("ssh-pass")
    );
    let deleted = service::delete_connection(&h.state, &saved.id)
        .await
        .unwrap();
    assert!(!deleted.password_left);
    assert_eq!(h.secrets.get(&ssh_secret_key(&saved.id)).unwrap(), None);
    assert_eq!(h.secrets.get(&password_key(&saved.id)).unwrap(), None);
}

#[tokio::test]
async fn remember_ssh_secret_marks_the_tunnel_as_saved() {
    let h = harness();
    let saved = service::save_connection(&h.state, draft(Some(tunnel(false)), None))
        .await
        .unwrap();
    assert!(saved.ssh.as_ref().unwrap().needs_prompt());
    service::remember_ssh_secret(&h.state, &saved.id, "typed".into())
        .await
        .unwrap();
    let config = h.state.find(&saved.id).unwrap();
    assert!(config.ssh.unwrap().save_secret);
    assert_eq!(
        h.state
            .saved_ssh_secret(&h.state.find(&saved.id).unwrap())
            .unwrap()
            .as_deref(),
        Some("typed")
    );

    let direct = service::save_connection(&h.state, draft(None, None))
        .await
        .unwrap();
    let error = service::remember_ssh_secret(&h.state, &direct.id, "x".into())
        .await
        .unwrap_err();
    assert_eq!(error.code, "unsupported");
}

#[test]
fn route_changes_ignore_the_save_flag() {
    let base = draft(Some(tunnel(true)), None)
        .into_config("c".into())
        .unwrap();
    let unsaved = ConnectionConfig {
        ssh: Some(tunnel(false)),
        ..base.clone()
    };
    assert!(!route_changed(&base, &unsaved));
    let other_host = ConnectionConfig {
        ssh: Some(SshTunnel {
            host: "other".into(),
            ..tunnel(true)
        }),
        ..base.clone()
    };
    assert!(route_changed(&base, &other_host));
    let ssl = ConnectionConfig {
        ssl: SslSettings {
            mode: SslMode::Required,
            ..SslSettings::default()
        },
        ..base.clone()
    };
    assert!(route_changed(&base, &ssl));
}

#[tokio::test]
async fn unknown_ssh_server_error_carries_the_host_key_to_the_frontend() {
    // 进程内起不了 SSH 服务器(那在 core 的隧道测试里);这里只看错误怎么转给前端、确认后记下。
    let host_key = HostKey {
        host: "jump".into(),
        port: 22,
        algorithm: "ssh-ed25519".into(),
        fingerprint: "SHA256:yJPhXHcJ/CHD/GzQdaieqdFuFbR4bHR7dhJZxohZ/0k".into(),
        public_key:
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINNMJeJ83kLKs6Vx+1LqlWrb+X2iS6OyXmpHnI3zDrbx"
                .into(),
    };
    let error = CommandError::from(CoreError::HostKeyUnknown(Box::new(host_key.clone())));
    assert_eq!(error.code, "host_key_unknown");
    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(json["hostKey"]["fingerprint"], host_key.fingerprint);
    let plain = serde_json::to_value(CommandError::from(CoreError::Timeout)).unwrap();
    assert!(plain.get("hostKey").is_none());

    let h = harness();
    service::trust_host_key(&h.state, &host_key).await.unwrap();
    let text = std::fs::read_to_string(h.state.known_hosts.path()).unwrap();
    assert!(text.contains("jump ssh-ed25519 AAAAC3"), "{text}");
    let tampered = HostKey {
        fingerprint: "SHA256:something-else".into(),
        ..host_key
    };
    assert!(service::trust_host_key(&h.state, &tampered).await.is_err());
}

#[tokio::test]
async fn test_connection_goes_through_the_tunnel_first() {
    let h = harness();
    // 隧道目标是一个不监听的本机端口:还没到登录就会失败,说明确实先走了隧道。
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let draft = draft(
        Some(SshTunnel {
            host: "127.0.0.1".into(),
            port,
            ..tunnel(false)
        }),
        None,
    );
    let error = service::test_connection(&h.state, draft).await.unwrap_err();
    assert_eq!(error.code, "ssh", "{}", error.message);
    assert!(error.message.contains("SSH 服务器"), "{}", error.message);
}

/// 和 core 的 tests/common 一样按 BASALT_IT 选:要同时选中 sshd 和 postgres。
fn sshd_and_postgres_enabled() -> bool {
    match std::env::var("BASALT_IT").as_deref().map(str::trim) {
        Ok("1" | "all") => true,
        Ok(list) => ["sshd", "postgres"]
            .iter()
            .all(|name| list.split(',').any(|n| n.trim() == *name)),
        Err(_) => false,
    }
}

/// 从表单测试、到核对指纹、到打开:全程经 docker-compose.test.yml 里的 sshd。
#[tokio::test]
async fn tunneled_connection_from_form_to_open_session() {
    if !sshd_and_postgres_enabled() {
        return;
    }
    let h = harness();
    let form = ConnectionDraft {
        driver: DriverKind::Postgres,
        endpoint: Endpoint::Network {
            host: "postgres".into(),
            port: 5432,
            username: "postgres".into(),
            database: None,
        },
        password: Some("basalt-test".into()),
        ..draft(
            Some(SshTunnel {
                host: "127.0.0.1".into(),
                port: 20022,
                username: "basalt".into(),
                ..tunnel(true)
            }),
            Some("basalt-test"),
        )
    };
    let error = service::test_connection(&h.state, form.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, "host_key_unknown", "{}", error.message);
    let host_key = error.host_key.expect("带着要核对的主机密钥");
    service::trust_host_key(&h.state, &host_key).await.unwrap();
    let server = service::test_connection(&h.state, form.clone())
        .await
        .unwrap();
    assert_eq!(server.product, "PostgreSQL");

    let saved = service::save_connection(&h.state, form).await.unwrap();
    let opened = service::open_connection(&h.state, &saved.id, None, None)
        .await
        .unwrap();
    assert_eq!(opened.product, "PostgreSQL");
    let databases = service::list_databases(&h.state, &saved.id).await.unwrap();
    assert!(databases.iter().any(|d| d == "postgres"));

    // 换了隧道设置:已打开的会话(连同隧道)关掉,下次按新设置连。
    let moved = ConnectionDraft {
        id: Some(saved.id.clone()),
        driver: DriverKind::Postgres,
        endpoint: saved.endpoint.clone(),
        password: None,
        ..draft(
            Some(SshTunnel {
                port: 20023,
                ..saved.ssh.clone().unwrap()
            }),
            None,
        )
    };
    service::save_connection(&h.state, moved).await.unwrap();
    assert!(!h.state.sessions.is_open(&saved.id).await);
}
