//! Redis 单机的连接:密码与 ACL 用户、库列表、TLS 各档、经 SSH 跳板机、连接断了自动重连。
//! 需要 BASALT_IT 选中 redis(经跳板机的还要选中 sshd),见 common。

mod common;

use basalt_core::config::{SslMode, SslSettings};
use basalt_core::driver::ConnectParams;
use basalt_core::redis_db::RedisSession;
use basalt_core::session::{SessionManager, test_connection};
use basalt_core::tunnel::{KnownHosts, SshAuth, SshParams};
use common::*;

fn redis(password: Option<&str>) -> ConnectParams {
    ConnectParams {
        password: password.map(str::to_owned),
        ..params(REDIS, TEST_PASSWORD, None)
    }
}

fn tls(mode: SslMode, host: &str, ca: Option<&str>) -> ConnectParams {
    ConnectParams {
        host: host.into(),
        port: REDIS_TLS_PORT,
        ssl: SslSettings {
            mode,
            ca_path: ca.map(fixture),
            ..SslSettings::default()
        },
        ..redis(Some(TEST_PASSWORD))
    }
}

/// 直接在容器上写测试数据(不经过 Basalt)。
async fn raw(db: u32) -> redis::aio::MultiplexedConnection {
    let url = format!("redis://:{TEST_PASSWORD}@127.0.0.1:{}/{db}", REDIS.port);
    redis::Client::open(url)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .expect("连不上 Redis 测试容器")
}

#[tokio::test]
async fn connects_with_the_default_user_password() {
    if !selected(REDIS) {
        return;
    }
    let server = test_connection(&redis(Some(TEST_PASSWORD))).await.unwrap();
    assert_eq!(server.product, "Redis");
    assert!(server.version.starts_with("7."), "{}", server.version);
}

#[tokio::test]
async fn a_wrong_or_missing_password_asks_for_the_password_again() {
    if !selected(REDIS) {
        return;
    }
    for password in [Some("nope"), None] {
        let error = test_connection(&redis(password)).await.unwrap_err();
        assert_eq!(error.code(), "authentication", "{password:?}: {error}");
    }
}

#[tokio::test]
async fn acl_users_log_in_with_their_name() {
    if !selected(REDIS) {
        return;
    }
    let user = |password: &str| ConnectParams {
        username: "basalt".into(),
        ..redis(Some(password))
    };
    test_connection(&user("basalt-user")).await.unwrap();
    let error = test_connection(&user(TEST_PASSWORD)).await.unwrap_err();
    assert_eq!(error.code(), "authentication", "{error}");
}

#[tokio::test]
async fn lists_every_database_with_its_key_counts() {
    if !selected(REDIS) {
        return;
    }
    let mut db7 = raw(7).await;
    let _: () = redis::cmd("SET")
        .arg("basalt:it:keyspace")
        .arg("1")
        .query_async(&mut db7)
        .await
        .unwrap();
    let mut db9 = raw(9).await;
    let _: () = redis::cmd("SET")
        .arg("basalt:it:expiring")
        .arg("1")
        .arg("EX")
        .arg(600)
        .query_async(&mut db9)
        .await
        .unwrap();

    let session = RedisSession::open(&ConnectParams {
        database: Some("9".into()),
        ..redis(Some(TEST_PASSWORD))
    })
    .await
    .unwrap();
    let keyspace = session.keyspace().await.unwrap();
    assert_eq!(keyspace.count, 16);
    assert_eq!(keyspace.default_db, 9);
    let indexes: Vec<u32> = keyspace.databases.iter().map(|d| d.index).collect();
    assert_eq!(indexes, (0..16).collect::<Vec<_>>());
    assert!(keyspace.databases[7].keys >= 1);
    assert!(keyspace.databases[9].expires >= 1);
    session.close().await;
}

#[tokio::test]
async fn tls_checks_as_much_as_the_mode_asks() {
    if !selected(REDIS) {
        return;
    }
    // 「必须」只加密:测试 CA 不是公共 CA,照样连得上。
    test_connection(&tls(SslMode::Required, "127.0.0.1", None))
        .await
        .unwrap();
    test_connection(&tls(SslMode::VerifyCa, "127.0.0.1", Some("tls/ca.crt")))
        .await
        .unwrap();
    test_connection(&tls(
        SslMode::VerifyIdentity,
        "localhost",
        Some("tls/ca.crt"),
    ))
    .await
    .unwrap();

    let error = test_connection(&tls(
        SslMode::VerifyCa,
        "127.0.0.1",
        Some("tls/other-ca.crt"),
    ))
    .await
    .unwrap_err();
    assert_eq!(error.code(), "network", "{error}");
    assert!(error.to_string().contains("所选的 CA"), "{error}");
    // 没选 CA 时只认公共 CA,测试 CA 签的证书过不了。
    let error = test_connection(&tls(SslMode::VerifyIdentity, "localhost", None))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "network", "{error}");
}

#[tokio::test]
async fn tls_sends_the_client_certificate_when_given() {
    if !selected(REDIS) {
        return;
    }
    let mut params = tls(SslMode::VerifyCa, "127.0.0.1", Some("tls/ca.crt"));
    params.ssl.client_cert_path = Some(fixture("tls/client.crt"));
    params.ssl.client_key_path = Some(fixture("tls/client.key"));
    test_connection(&params).await.unwrap();
}

fn via_jump(port: u16, known_hosts: &KnownHosts) -> ConnectParams {
    ConnectParams {
        host: "redis".into(),
        port,
        ssh: Some(SshParams {
            host: "127.0.0.1".into(),
            port: SSHD_PORT,
            username: SSHD_USER.into(),
            auth: SshAuth::Password(Some(TEST_PASSWORD.into())),
            jumps: Vec::new(),
            agent: None,
            known_hosts: known_hosts.clone(),
        }),
        ..redis(Some(TEST_PASSWORD))
    }
}

/// 第一次连接拿到主机密钥 → 像用户点了「信任」那样记下 → 再连。
async fn test_trusting(params: &ConnectParams) -> basalt_core::driver::ServerInfo {
    match test_connection(params).await {
        Ok(server) => server,
        Err(error) if error.code() == "host_key_unknown" => {
            let host_key = error.host_key().expect("带着主机密钥").clone();
            params
                .ssh
                .as_ref()
                .unwrap()
                .known_hosts
                .trust(&host_key)
                .unwrap();
            test_connection(params).await.expect("信任之后应该能连上")
        }
        Err(error) => panic!("经隧道连不上：{error}"),
    }
}

#[tokio::test]
async fn reaches_redis_through_the_jump_host_with_and_without_tls() {
    if !(sshd_selected() && selected(REDIS)) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = KnownHosts::new(dir.path().join("known_hosts"));
    assert_eq!(
        test_trusting(&via_jump(6379, &known_hosts)).await.product,
        "Redis"
    );

    // 证书里没有「redis」这个名字:校验 CA 能过,经隧道也照原主机名核对,所以校验主机名过不了。
    let mut params = via_jump(6380, &known_hosts);
    params.ssl = SslSettings {
        mode: SslMode::VerifyCa,
        ca_path: Some(fixture("tls/ca.crt")),
        ..SslSettings::default()
    };
    test_connection(&params).await.unwrap();
    params.ssl.mode = SslMode::VerifyIdentity;
    let error = test_connection(&params).await.unwrap_err();
    assert!(error.to_string().contains("主机名"), "{error}");
}

#[tokio::test]
async fn the_manager_keeps_redis_apart_from_sql_sessions() {
    if !selected(REDIS) {
        return;
    }
    let manager = SessionManager::default();
    let server = manager
        .connect("r1", &redis(Some(TEST_PASSWORD)))
        .await
        .unwrap();
    assert_eq!(server.product, "Redis");
    assert!(manager.is_open("r1").await);
    assert_eq!(manager.get("r1").await.unwrap_err().code(), "unsupported");
    manager
        .get_redis("r1")
        .await
        .unwrap()
        .keyspace()
        .await
        .unwrap();
    assert!(manager.close("r1").await);
    assert_eq!(
        manager.get_redis("r1").await.unwrap_err().code(),
        "not_connected"
    );
}

#[tokio::test]
async fn opens_a_new_connection_when_the_old_one_was_killed() {
    if !selected(REDIS) {
        return;
    }
    let session = RedisSession::open(&ConnectParams {
        database: Some("11".into()),
        ..redis(Some(TEST_PASSWORD))
    })
    .await
    .unwrap();
    session.keyspace().await.unwrap();
    // 只踢掉这个会话在 11 号库上的连接,不碰别的测试。
    let mut admin = raw(0).await;
    let clients: String = redis::cmd("CLIENT")
        .arg("LIST")
        .query_async(&mut admin)
        .await
        .unwrap();
    let ids: Vec<&str> = clients
        .lines()
        .filter(|line| line.contains(" db=11 ") && line.contains("lib-name=basalt"))
        .filter_map(|line| line.split(' ').find_map(|field| field.strip_prefix("id=")))
        .collect();
    assert!(!ids.is_empty(), "{clients}");
    for id in ids {
        let _: i64 = redis::cmd("CLIENT")
            .arg("KILL")
            .arg("ID")
            .arg(id)
            .query_async(&mut admin)
            .await
            .unwrap();
    }
    session
        .keyspace()
        .await
        .expect("连接被踢掉后应该自己换一条");
    session.close().await;
}
