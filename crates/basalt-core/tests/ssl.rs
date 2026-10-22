//! MySQL 5.7 / PostgreSQL 的 SSL 模式、CA 校验和客户端证书登录。
//! 两个容器挂着测试 CA 签的证书(tests/fixtures/tls),需要 BASALT_IT 选中对应的库,见 common。

mod common;

use basalt_core::config::{SslMode, SslSettings};
use basalt_core::driver::ConnectParams;
use basalt_core::query::{OutcomeStatus, QueryScope};
use basalt_core::session::{Session, SessionManager};
use basalt_core::value::CellValue;
use common::*;

/// 用证书登录的账号:PG 的角色名就是客户端证书的 CN;MySQL 的账号要求 X509。
const CERT_USER: &str = "basalt_cert";

fn ssl(mode: SslMode, ca: Option<&str>) -> SslSettings {
    SslSettings {
        mode,
        ca_path: ca.map(|name| fixture(&format!("tls/{name}"))),
        ..SslSettings::default()
    }
}

fn with_client_cert(settings: SslSettings) -> SslSettings {
    SslSettings {
        client_cert_path: Some(fixture("tls/client.crt")),
        client_key_path: Some(fixture("tls/client.key")),
        ..settings
    }
}

fn direct(target: Target, settings: SslSettings) -> ConnectParams {
    ConnectParams {
        ssl: settings,
        ..params(target, TEST_PASSWORD, None)
    }
}

/// 连上以后问服务器:这条连接是不是 TLS。
async fn encrypted(params: &ConnectParams) -> bool {
    let manager = SessionManager::default();
    manager.open("c1", params).await.unwrap();
    let sql = match params.driver {
        basalt_core::config::DriverKind::Postgres => {
            "SELECT ssl::text FROM pg_stat_ssl WHERE pid = pg_backend_pid()"
        }
        _ => "SHOW SESSION STATUS LIKE 'Ssl_cipher'",
    };
    let outcome = manager
        .run_query("tab", "c1", &QueryScope::default(), sql)
        .await
        .unwrap();
    manager.close("c1").await;
    assert_eq!(outcome.status, OutcomeStatus::Ok, "{:?}", outcome.message);
    let row = &outcome.result_sets[0].rows[0];
    match (params.driver, row.last()) {
        (basalt_core::config::DriverKind::Postgres, Some(CellValue::Text(v))) => v == "true",
        (_, Some(CellValue::Text(cipher))) => !cipher.is_empty(),
        (_, other) => panic!("看不懂的结果：{other:?}"),
    }
}

async fn connect_error(params: &ConnectParams) -> basalt_core::CoreError {
    Session::open(params).await.unwrap_err()
}

#[tokio::test]
async fn postgres_ssl_modes() {
    if !selected(POSTGRES) {
        return;
    }
    assert!(!encrypted(&direct(POSTGRES, ssl(SslMode::Disabled, None))).await);
    assert!(encrypted(&direct(POSTGRES, ssl(SslMode::Preferred, None))).await);
    assert!(encrypted(&direct(POSTGRES, ssl(SslMode::Required, None))).await);
    assert!(encrypted(&direct(POSTGRES, ssl(SslMode::VerifyCa, Some("ca.crt")))).await);
    // 证书里有 IP 127.0.0.1,核对主机名也能过。
    assert!(
        encrypted(&direct(
            POSTGRES,
            ssl(SslMode::VerifyIdentity, Some("ca.crt"))
        ))
        .await
    );
}

#[tokio::test]
async fn mysql_ssl_modes() {
    if !selected(MYSQL57) {
        return;
    }
    assert!(!encrypted(&direct(MYSQL57, ssl(SslMode::Disabled, None))).await);
    assert!(encrypted(&direct(MYSQL57, ssl(SslMode::Preferred, None))).await);
    assert!(encrypted(&direct(MYSQL57, ssl(SslMode::Required, None))).await);
    assert!(encrypted(&direct(MYSQL57, ssl(SslMode::VerifyCa, Some("ca.crt")))).await);
    assert!(
        encrypted(&direct(
            MYSQL57,
            ssl(SslMode::VerifyIdentity, Some("ca.crt"))
        ))
        .await
    );
}

#[tokio::test]
async fn certificates_from_another_ca_are_rejected() {
    for target in [POSTGRES, MYSQL57] {
        if !selected(target) {
            continue;
        }
        let error = connect_error(&direct(
            target,
            ssl(SslMode::VerifyCa, Some("other-ca.crt")),
        ))
        .await;
        assert!(
            error.to_string().contains("不是由所选的 CA 签发的"),
            "{}: {error}",
            target.name
        );
        // 不给 CA 时用内置的公共 CA 列表,测试 CA 当然不在里面。
        let error = connect_error(&direct(target, ssl(SslMode::VerifyCa, None))).await;
        assert!(
            error.to_string().contains("TLS"),
            "{}: {error}",
            target.name
        );
    }
}

#[tokio::test]
async fn postgres_client_certificate_login() {
    if !selected(POSTGRES) {
        return;
    }
    let admin = sqlx::PgPool::connect(&postgres_url("postgres"))
        .await
        .unwrap();
    sqlx::raw_sql(
        "DO $$ BEGIN
           IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'basalt_cert') THEN
             CREATE ROLE basalt_cert LOGIN;
           END IF;
         END $$",
    )
    .execute(&admin)
    .await
    .unwrap();
    admin.close().await;

    let as_cert_user = |settings| ConnectParams {
        username: CERT_USER.into(),
        password: None,
        ..direct(POSTGRES, settings)
    };
    let session = Session::open(&as_cert_user(with_client_cert(ssl(
        SslMode::VerifyCa,
        Some("ca.crt"),
    ))))
    .await
    .expect("用客户端证书登录");
    session.close().await;
    // 没带证书:pg_hba 只认证书,拒绝。
    connect_error(&as_cert_user(ssl(SslMode::VerifyCa, Some("ca.crt")))).await;
}

#[tokio::test]
async fn mysql_client_certificate_login() {
    if !selected(MYSQL57) {
        return;
    }
    let admin = sqlx::MySqlPool::connect(&mysql_url(MYSQL57)).await.unwrap();
    sqlx::raw_sql(
        "CREATE USER IF NOT EXISTS 'basalt_cert'@'%' IDENTIFIED BY 'basalt-test' REQUIRE X509",
    )
    .execute(&admin)
    .await
    .unwrap();
    admin.close().await;

    let as_cert_user = |settings| ConnectParams {
        username: CERT_USER.into(),
        ..direct(MYSQL57, settings)
    };
    let session = Session::open(&as_cert_user(with_client_cert(ssl(
        SslMode::Required,
        None,
    ))))
    .await
    .expect("带客户端证书登录");
    session.close().await;
    let error = connect_error(&as_cert_user(ssl(SslMode::Required, None))).await;
    assert_eq!(error.code(), "authentication", "{error}");
}

#[tokio::test]
async fn missing_certificate_files_are_reported_before_connecting() {
    for target in [POSTGRES, MYSQL57] {
        if !selected(target) {
            continue;
        }
        let missing = SslSettings {
            ca_path: Some("/nonexistent/basalt-ca.crt".into()),
            ..ssl(SslMode::VerifyCa, None)
        };
        let error = connect_error(&direct(target, missing)).await;
        assert_eq!(error.code(), "file", "{error}");
    }
}
