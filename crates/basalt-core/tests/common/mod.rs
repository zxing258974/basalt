//! 集成测试共用的测试目标与连接参数。
//!
//! MySQL / MariaDB / PostgreSQL 需要先起 `docker-compose.test.yml` 里的容器,
//! 并用 `BASALT_IT` 选择目标:`BASALT_IT=1` 等于 `mysql57,postgres,sshd`,
//! `BASALT_IT=all` 是全部,也可以写逗号分隔的名单。没选中的目标直接跳过。
//! sshd 是 SSH 跳板机,经它连库的测试同时要选中 sshd 和那个库。
#![allow(dead_code)]

use std::path::Path;

use basalt_core::config::DriverKind;
use basalt_core::driver::ConnectParams;

/// 本机一次性测试容器的密码,与 docker-compose.test.yml 一致。
pub const TEST_PASSWORD: &str = "basalt-test";
pub const FIXTURE_DB: &str = "basalt_it";

#[derive(Clone, Copy)]
pub struct Target {
    pub name: &'static str,
    pub driver: DriverKind,
    pub port: u16,
    pub user: &'static str,
}

pub const MYSQL57: Target = Target {
    name: "mysql57",
    driver: DriverKind::Mysql,
    port: 23306,
    user: "root",
};
pub const MYSQL84: Target = Target {
    name: "mysql84",
    driver: DriverKind::Mysql,
    port: 23307,
    user: "root",
};
pub const MARIADB: Target = Target {
    name: "mariadb",
    driver: DriverKind::Mariadb,
    port: 23308,
    user: "root",
};
pub const POSTGRES: Target = Target {
    name: "postgres",
    driver: DriverKind::Postgres,
    port: 25432,
    user: "postgres",
};
pub const MYSQL_FAMILY: [Target; 3] = [MYSQL57, MYSQL84, MARIADB];

pub fn selected(target: Target) -> bool {
    selected_name(target.name)
}

fn selected_name(name: &str) -> bool {
    let Ok(raw) = std::env::var("BASALT_IT") else {
        return false;
    };
    match raw.trim() {
        "1" => matches!(name, "mysql57" | "postgres" | "sshd"),
        "all" => true,
        list => list.split(',').any(|n| n.trim() == name),
    }
}

/// docker-compose.test.yml 里的 SSH 跳板机:只监听本机 20022,用户 basalt / noforward(不许转发)。
pub const SSHD_PORT: u16 = 20022;
pub const SSHD_USER: &str = "basalt";
pub const SSHD_NO_FORWARD_USER: &str = "noforward";

pub fn sshd_selected() -> bool {
    selected_name("sshd")
}

/// tests/fixtures 下某个文件的绝对路径。
pub fn fixture(relative: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
        .to_string_lossy()
        .into_owned()
}

pub fn params(target: Target, password: &str, database: Option<&str>) -> ConnectParams {
    ConnectParams {
        driver: target.driver,
        host: "127.0.0.1".into(),
        port: target.port,
        username: target.user.into(),
        password: Some(password.into()),
        database: database.map(str::to_owned),
        file_path: None,
        ssl: basalt_core::config::SslSettings::default(),
        ssh: None,
    }
}

/// MySQL 系目标的连接串(不带库名)。
pub fn mysql_url(target: Target) -> String {
    format!(
        "mysql://{}:{TEST_PASSWORD}@127.0.0.1:{}/",
        target.user, target.port
    )
}

pub fn postgres_url(database: &str) -> String {
    format!(
        "postgres://postgres:{TEST_PASSWORD}@127.0.0.1:{}/{database}",
        POSTGRES.port
    )
}

pub fn sqlite_params(path: &str) -> ConnectParams {
    ConnectParams {
        driver: DriverKind::Sqlite,
        host: String::new(),
        port: 0,
        username: String::new(),
        password: None,
        database: None,
        file_path: Some(path.into()),
        ssl: basalt_core::config::SslSettings::default(),
        ssh: None,
    }
}

/// 建一个 SQLite 文件并执行建表脚本,返回路径。
pub async fn sqlite_file(dir: &Path, name: &str, script: &str) -> String {
    let path = dir.join(name);
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(script))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    path.to_string_lossy().into_owned()
}

/// 连上 PostgreSQL 的测试库;单独跑某个测试文件时库可能还没建,先建出来。
pub async fn postgres_fixture_pool() -> sqlx::PgPool {
    if let Ok(pool) = sqlx::PgPool::connect(&postgres_url(FIXTURE_DB)).await {
        return pool;
    }
    let admin = sqlx::PgPool::connect(&postgres_url("postgres"))
        .await
        .unwrap();
    // 别的测试文件可能同时在建:已经有了不算错。
    let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {FIXTURE_DB}")))
        .execute(&admin)
        .await;
    admin.close().await;
    sqlx::PgPool::connect(&postgres_url(FIXTURE_DB))
        .await
        .unwrap()
}
