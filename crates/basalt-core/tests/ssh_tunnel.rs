//! 经 docker-compose.test.yml 里的 sshd 跳板机连 MySQL 5.7 和 PostgreSQL(M4 的完成标准)。
//! 需要 BASALT_IT 同时选中 sshd 和对应的库,见 common。数据库地址写容器名:那是从跳板机看过去的地址。

mod common;

use basalt_core::config::{SslMode, SslSettings};
use basalt_core::driver::ConnectParams;
use basalt_core::query::{OutcomeStatus, QueryScope};
use basalt_core::session::{Session, SessionManager};
use basalt_core::tunnel::{KnownHosts, SshAuth, SshParams};
use basalt_core::value::CellValue;
use common::*;

/// 每个测试用自己的 known_hosts:第一次连接都要走「核对指纹 → 信任」。
struct Jump {
    _dir: tempfile::TempDir,
    known_hosts: KnownHosts,
}

fn jump() -> Jump {
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = KnownHosts::new(dir.path().join("known_hosts"));
    Jump {
        _dir: dir,
        known_hosts,
    }
}

fn password() -> SshAuth {
    SshAuth::Password(Some(TEST_PASSWORD.into()))
}

fn key(name: &str, passphrase: Option<&str>) -> SshAuth {
    SshAuth::PrivateKey {
        path: fixture(&format!("ssh/{name}")),
        passphrase: passphrase.map(str::to_owned),
    }
}

/// 经跳板机连 `target`:数据库地址换成容器名和容器里的端口。
fn via_jump(target: Target, jump: &Jump, user: &str, auth: SshAuth) -> ConnectParams {
    let (host, port) = match target.name {
        "postgres" => ("postgres", 5432),
        _ => ("mysql57", 3306),
    };
    ConnectParams {
        host: host.into(),
        port,
        ssh: Some(SshParams {
            host: "127.0.0.1".into(),
            port: SSHD_PORT,
            username: user.into(),
            auth,
            known_hosts: jump.known_hosts.clone(),
        }),
        ..params(target, TEST_PASSWORD, None)
    }
}

fn enabled(target: Target) -> bool {
    sshd_selected() && selected(target)
}

/// 第一次连接拿到主机密钥 → 像用户点了「信任」那样记下 → 再连。
async fn open_trusting(params: &ConnectParams) -> Session {
    match Session::open(params).await {
        Ok(session) => session,
        Err(error) if error.code() == "host_key_unknown" => {
            let host_key = error.host_key().expect("带着主机密钥").clone();
            assert!(host_key.fingerprint.starts_with("SHA256:"));
            assert_eq!(host_key.port, SSHD_PORT);
            params
                .ssh
                .as_ref()
                .unwrap()
                .known_hosts
                .trust(&host_key)
                .unwrap();
            Session::open(params).await.expect("信任之后应该能连上")
        }
        Err(error) => panic!("经隧道连不上：{error}"),
    }
}

/// 在查询标签页上执行一句,取第一行第一列。
async fn scalar(manager: &SessionManager, sql: &str) -> String {
    let outcome = manager
        .run_query("tab", "c1", &QueryScope::default(), sql)
        .await
        .unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Ok, "{:?}", outcome.message);
    match &outcome.result_sets[0].rows[0][0] {
        CellValue::Text(text) | CellValue::Decimal(text) => text.clone(),
        CellValue::Int(n) => n.to_string(),
        CellValue::Bool(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}

#[tokio::test]
async fn mysql_through_the_jump_host_with_a_password() {
    if !enabled(MYSQL57) {
        return;
    }
    let jump = jump();
    let params = via_jump(MYSQL57, &jump, SSHD_USER, password());
    let session = open_trusting(&params).await;
    assert_eq!(session.server().product, "MySQL");
    assert!(session.server().version.starts_with("5.7"));
    let databases = session.list_databases().await.unwrap();
    assert!(databases.iter().any(|d| d == "mysql"), "{databases:?}");
    session.close().await;

    // 信任过了,第二次直接连上;查询标签页的专用连接也走隧道。
    let manager = SessionManager::default();
    manager.open("c1", &params).await.unwrap();
    // 隧道那头看到的客户端是跳板机,不是本机:连接来自 sshd 容器。
    let user = scalar(&manager, "SELECT CURRENT_USER()").await;
    assert!(user.starts_with("root@"), "{user}");
    assert_eq!(scalar(&manager, "SELECT 40 + 2").await, "42");
    manager.close("c1").await;
}

#[tokio::test]
async fn postgres_through_the_jump_host_with_each_kind_of_private_key() {
    if !enabled(POSTGRES) {
        return;
    }
    let jump = jump();
    for (name, passphrase) in [
        ("id_ed25519", None),
        ("id_ed25519_passphrase", Some("basalt-key-pass")),
        ("id_rsa_pem", None),
    ] {
        let params = via_jump(POSTGRES, &jump, SSHD_USER, key(name, passphrase));
        let session = open_trusting(&params).await;
        assert_eq!(session.server().product, "PostgreSQL", "{name}");
        // 换库要新建连接池,新池子的连接同样经隧道。
        let schemas = session.list_schemas("template1").await.unwrap();
        assert!(schemas.iter().any(|s| s == "public"), "{name}: {schemas:?}");
        session.close().await;
    }
}

#[tokio::test]
async fn tunnel_and_ssl_together() {
    if !enabled(POSTGRES) {
        return;
    }
    let jump = jump();
    let params = ConnectParams {
        ssl: SslSettings {
            mode: SslMode::VerifyCa,
            ca_path: Some(fixture("tls/ca.crt")),
            ..SslSettings::default()
        },
        ..via_jump(POSTGRES, &jump, SSHD_USER, password())
    };
    open_trusting(&params).await.close().await;
    let manager = SessionManager::default();
    manager.open("c1", &params).await.unwrap();
    let ssl = scalar(
        &manager,
        "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
    )
    .await;
    assert_eq!(ssl, "true", "隧道里那一段也是 TLS");
    manager.close("c1").await;
}

#[tokio::test]
async fn login_and_forwarding_failures_are_explained() {
    if !enabled(MYSQL57) {
        return;
    }
    let jump = jump();
    // 先信任一次,后面的失败才走到登录和转发。
    open_trusting(&via_jump(MYSQL57, &jump, SSHD_USER, password()))
        .await
        .close()
        .await;

    let wrong = via_jump(
        MYSQL57,
        &jump,
        SSHD_USER,
        SshAuth::Password(Some("nope".into())),
    );
    let error = Session::open(&wrong).await.unwrap_err();
    assert_eq!(error.code(), "ssh_authentication", "{error}");

    let missing = via_jump(MYSQL57, &jump, SSHD_USER, SshAuth::Password(None));
    let error = Session::open(&missing).await.unwrap_err();
    assert_eq!(error.code(), "ssh_secret_required", "{error}");

    let no_passphrase = via_jump(
        MYSQL57,
        &jump,
        SSHD_USER,
        key("id_ed25519_passphrase", None),
    );
    let error = Session::open(&no_passphrase).await.unwrap_err();
    assert_eq!(error.code(), "ssh_secret_required", "{error}");

    let prohibited = via_jump(MYSQL57, &jump, SSHD_NO_FORWARD_USER, password());
    let error = Session::open(&prohibited).await.unwrap_err();
    assert_eq!(error.code(), "ssh", "{error}");
    assert!(error.to_string().contains("AllowTcpForwarding"), "{error}");

    let nowhere = ConnectParams {
        host: "no-such-db.invalid".into(),
        ..via_jump(MYSQL57, &jump, SSHD_USER, password())
    };
    let error = Session::open(&nowhere).await.unwrap_err();
    assert_eq!(error.code(), "ssh", "{error}");
    assert!(
        error.to_string().contains("连不上 no-such-db.invalid"),
        "{error}"
    );

    // 数据库密码不对:还是数据库的认证错误,前端照常重新问数据库密码。
    let bad_db_password = ConnectParams {
        password: Some("nope".into()),
        ..via_jump(MYSQL57, &jump, SSHD_USER, password())
    };
    let error = Session::open(&bad_db_password).await.unwrap_err();
    assert_eq!(error.code(), "authentication", "{error}");
}

#[tokio::test]
async fn changed_host_key_is_refused_against_the_real_server() {
    if !enabled(MYSQL57) {
        return;
    }
    let jump = jump();
    // 冒充「上次记下的」主机密钥:拿客户端那把公钥当成这台服务器的。
    let fake = std::fs::read_to_string(fixture("ssh/id_ed25519.pub")).unwrap();
    let line = format!("[127.0.0.1]:{SSHD_PORT} {}\n", fake.trim());
    std::fs::write(jump.known_hosts.path(), line).unwrap();
    let error = Session::open(&via_jump(MYSQL57, &jump, SSHD_USER, password()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "host_key_changed", "{error}");
    assert!(error.to_string().contains("第 1 行"), "{error}");
}

/// 经隧道时驱动连的是只有当前用户能进的临时目录里的 Unix 套接字,TLS 照样协商、照样校验 CA。
#[tokio::test]
async fn mysql_through_the_jump_host_with_tls_over_the_local_socket() {
    if !enabled(MYSQL57) {
        return;
    }
    let jump = jump();
    let params = ConnectParams {
        ssl: SslSettings {
            mode: SslMode::VerifyCa,
            ca_path: Some(fixture("tls/ca.crt")),
            ..SslSettings::default()
        },
        ..via_jump(MYSQL57, &jump, SSHD_USER, password())
    };
    open_trusting(&params).await.close().await;
    let manager = SessionManager::default();
    manager.open("c1", &params).await.unwrap();
    let outcome = manager
        .run_query(
            "tab",
            "c1",
            &QueryScope::default(),
            "SHOW SESSION STATUS LIKE 'Ssl_cipher'",
        )
        .await
        .unwrap();
    // 第二列是 Ssl_cipher 的值,非空说明这条连接是 TLS。
    assert!(
        matches!(&outcome.result_sets[0].rows[0][1], CellValue::Text(v) if !v.is_empty()),
        "{:?}",
        outcome.result_sets[0].rows
    );
    manager.close("c1").await;
}
