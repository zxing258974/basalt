//! 驱动集成测试。
//!
//! SQLite 的用例总会运行;MySQL / MariaDB / PostgreSQL 需要先起
//! `docker-compose.test.yml` 里的容器,并用 `BASALT_IT` 选择目标:
//! `BASALT_IT=1` 等于 `mysql57,postgres`,`BASALT_IT=all` 是全部四个,
//! 也可以写逗号分隔的名单。没选中的目标直接跳过。

use std::path::Path;
use std::sync::Arc;

use basalt_core::config::DriverKind;
use basalt_core::driver::{ConnectParams, TableInfo, TableKind};
use basalt_core::session::{Session, SessionManager, test_connection};
use tokio::sync::OnceCell;

/// 本机一次性测试容器的密码,与 docker-compose.test.yml 一致。
const TEST_PASSWORD: &str = "basalt-test";
const FIXTURE_DB: &str = "basalt_it";

#[derive(Clone, Copy)]
struct Target {
    name: &'static str,
    driver: DriverKind,
    port: u16,
    user: &'static str,
}

const MYSQL57: Target = Target {
    name: "mysql57",
    driver: DriverKind::Mysql,
    port: 23306,
    user: "root",
};
const MYSQL84: Target = Target {
    name: "mysql84",
    driver: DriverKind::Mysql,
    port: 23307,
    user: "root",
};
const MARIADB: Target = Target {
    name: "mariadb",
    driver: DriverKind::Mariadb,
    port: 23308,
    user: "root",
};
const POSTGRES: Target = Target {
    name: "postgres",
    driver: DriverKind::Postgres,
    port: 25432,
    user: "postgres",
};
const MYSQL_FAMILY: [Target; 3] = [MYSQL57, MYSQL84, MARIADB];

fn selected(target: Target) -> bool {
    let Ok(raw) = std::env::var("BASALT_IT") else {
        return false;
    };
    match raw.trim() {
        "1" => matches!(target.name, "mysql57" | "postgres"),
        "all" => true,
        list => list.split(',').any(|n| n.trim() == target.name),
    }
}

fn params(target: Target, password: &str, database: Option<&str>) -> ConnectParams {
    ConnectParams {
        driver: target.driver,
        host: "127.0.0.1".into(),
        port: target.port,
        username: target.user.into(),
        password: Some(password.into()),
        database: database.map(str::to_owned),
        file_path: None,
    }
}

static MYSQL_FIXTURES: [OnceCell<()>; 3] = [
    OnceCell::const_new(),
    OnceCell::const_new(),
    OnceCell::const_new(),
];
static PG_FIXTURE: OnceCell<()> = OnceCell::const_new();

async fn mysql_fixture(index: usize, target: Target) {
    MYSQL_FIXTURES[index]
        .get_or_init(|| async move {
            let url = format!(
                "mysql://{}:{TEST_PASSWORD}@127.0.0.1:{}/",
                target.user, target.port
            );
            let pool = sqlx::MySqlPool::connect(&url)
                .await
                .expect("连接 MySQL 测试容器");
            sqlx::raw_sql(
                "CREATE DATABASE IF NOT EXISTS basalt_it CHARACTER SET utf8mb4;
                 CREATE TABLE IF NOT EXISTS basalt_it.users (
                   id INT PRIMARY KEY AUTO_INCREMENT,
                   name VARCHAR(50) NOT NULL COMMENT '姓名',
                   active TINYINT(1) NOT NULL DEFAULT 1
                 ) ENGINE=InnoDB COMMENT='用户表';
                 CREATE OR REPLACE VIEW basalt_it.active_users AS
                   SELECT id, name FROM basalt_it.users WHERE active = 1;",
            )
            .execute(&pool)
            .await
            .expect("建 MySQL 测试数据");
            pool.close().await;
        })
        .await;
}

async fn pg_fixture() {
    PG_FIXTURE
        .get_or_init(|| async {
            let base = format!("postgres://postgres:{TEST_PASSWORD}@127.0.0.1:{}", POSTGRES.port);
            let admin = sqlx::PgPool::connect(&format!("{base}/postgres")).await.expect("连接 PG 测试容器");
            let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
                .bind(FIXTURE_DB)
                .fetch_optional(&admin)
                .await
                .unwrap();
            if exists.is_none() {
                sqlx::raw_sql("CREATE DATABASE basalt_it").execute(&admin).await.unwrap();
            }
            admin.close().await;

            let db = sqlx::PgPool::connect(&format!("{base}/{FIXTURE_DB}")).await.unwrap();
            sqlx::raw_sql(
                "CREATE SCHEMA IF NOT EXISTS sales;
                 CREATE TABLE IF NOT EXISTS public.users (id serial PRIMARY KEY, name text NOT NULL);
                 COMMENT ON TABLE public.users IS '用户表';
                 CREATE OR REPLACE VIEW public.active_users AS SELECT id, name FROM public.users;
                 CREATE TABLE IF NOT EXISTS sales.orders (id bigserial PRIMARY KEY, amount numeric(10,2));
                 CREATE MATERIALIZED VIEW IF NOT EXISTS sales.order_totals AS SELECT count(*) AS n FROM sales.orders;",
            )
            .execute(&db)
            .await
            .expect("建 PG 测试数据");
            db.close().await;
        })
        .await;
}

fn find<'a>(tables: &'a [TableInfo], name: &str) -> &'a TableInfo {
    tables
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("缺少 {name}:{tables:?}"))
}

// ---------- MySQL / MariaDB ----------

#[tokio::test]
async fn mysql_family_reports_product_and_version() {
    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let info = test_connection(&params(target, TEST_PASSWORD, None))
            .await
            .unwrap();
        let expected = if target.driver == DriverKind::Mariadb {
            "MariaDB"
        } else {
            "MySQL"
        };
        assert_eq!(info.product, expected, "{}", target.name);
        assert!(!info.version.is_empty());
    }
}

#[tokio::test]
async fn mysql_family_lists_tables_and_views_with_metadata() {
    for (index, target) in MYSQL_FAMILY
        .into_iter()
        .enumerate()
        .filter(|(_, t)| selected(*t))
    {
        mysql_fixture(index, target).await;
        let session = Session::open(&params(target, TEST_PASSWORD, None))
            .await
            .unwrap();

        let databases = session.list_databases().await.unwrap();
        assert!(databases.iter().any(|d| d == FIXTURE_DB), "{databases:?}");
        assert!(session.list_schemas(FIXTURE_DB).await.unwrap().is_empty());

        let tables = session.list_tables(FIXTURE_DB, None).await.unwrap();
        let users = find(&tables, "users");
        assert_eq!(users.kind, TableKind::Table);
        assert_eq!(users.engine.as_deref(), Some("InnoDB"));
        assert_eq!(users.comment.as_deref(), Some("用户表"));
        assert!(users.data_bytes.is_some());

        let view = find(&tables, "active_users");
        assert_eq!(view.kind, TableKind::View);
        assert_eq!(
            view.comment, None,
            "MySQL 视图的 TABLE_COMMENT 是字面量 VIEW,不应显示"
        );
        session.close().await;
    }
}

#[tokio::test]
async fn mysql_family_wrong_password_is_authentication_error() {
    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let error = test_connection(&params(target, "wrong-password", None))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "authentication", "{}: {error}", target.name);
    }
}

// ---------- PostgreSQL ----------

#[tokio::test]
async fn postgres_reports_product_and_lists_databases() {
    if !selected(POSTGRES) {
        return;
    }
    pg_fixture().await;
    let session = Session::open(&params(POSTGRES, TEST_PASSWORD, None))
        .await
        .unwrap();
    assert_eq!(session.server().product, "PostgreSQL");

    let databases = session.list_databases().await.unwrap();
    assert!(databases.iter().any(|d| d == FIXTURE_DB), "{databases:?}");
    assert!(databases.iter().any(|d| d == "postgres"));
    assert!(!databases.iter().any(|d| d.starts_with("template")));
    session.close().await;
}

#[tokio::test]
async fn postgres_lists_only_user_schemas() {
    if !selected(POSTGRES) {
        return;
    }
    pg_fixture().await;
    let session = Session::open(&params(POSTGRES, TEST_PASSWORD, None))
        .await
        .unwrap();
    let schemas = session.list_schemas(FIXTURE_DB).await.unwrap();
    assert!(schemas.contains(&"public".to_owned()), "{schemas:?}");
    assert!(schemas.contains(&"sales".to_owned()));
    assert!(
        !schemas
            .iter()
            .any(|s| s.starts_with("pg_") || s == "information_schema")
    );
    session.close().await;
}

#[tokio::test]
async fn postgres_lists_relations_per_schema_in_other_database() {
    if !selected(POSTGRES) {
        return;
    }
    pg_fixture().await;
    // 初始库是 postgres,查询 basalt_it 需要会话按库另建连接。
    let session = Session::open(&params(POSTGRES, TEST_PASSWORD, Some("postgres")))
        .await
        .unwrap();

    let public = session
        .list_tables(FIXTURE_DB, Some("public"))
        .await
        .unwrap();
    let users = find(&public, "users");
    assert_eq!(users.kind, TableKind::Table);
    assert_eq!(users.comment.as_deref(), Some("用户表"));
    assert_eq!(find(&public, "active_users").kind, TableKind::View);

    let sales = session
        .list_tables(FIXTURE_DB, Some("sales"))
        .await
        .unwrap();
    assert_eq!(find(&sales, "orders").kind, TableKind::Table);
    assert_eq!(
        find(&sales, "order_totals").kind,
        TableKind::MaterializedView
    );
    assert!(sales.iter().all(|t| t.name != "users"));
    session.close().await;
}

#[tokio::test]
async fn postgres_wrong_password_is_authentication_error() {
    if !selected(POSTGRES) {
        return;
    }
    let error = test_connection(&params(POSTGRES, "wrong-password", None))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "authentication", "{error}");
}

// ---------- 与目标无关 ----------

#[tokio::test]
async fn closed_port_fails_fast_as_network_error() {
    for driver in [DriverKind::Mysql, DriverKind::Postgres] {
        let target = Target {
            port: 1,
            driver,
            ..MYSQL57
        };
        let started = std::time::Instant::now();
        let error = test_connection(&params(target, TEST_PASSWORD, None))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "network", "{driver:?}: {error}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "连接被拒绝应立即报错，不该等到超时：{:?}",
            started.elapsed()
        );
    }
}

// ---------- SQLite ----------

async fn sqlite_file(dir: &Path) -> String {
    let path = dir.join("notes.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL, created_at TEXT);
         CREATE VIEW recent_notes AS SELECT id, body FROM notes ORDER BY id DESC LIMIT 10;",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    path.to_string_lossy().into_owned()
}

fn sqlite_params(path: &str) -> ConnectParams {
    ConnectParams {
        driver: DriverKind::Sqlite,
        host: String::new(),
        port: 0,
        username: String::new(),
        password: None,
        database: None,
        file_path: Some(path.into()),
    }
}

#[tokio::test]
async fn sqlite_lists_main_database_tables_and_views() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path()).await;
    let session = Session::open(&sqlite_params(&path)).await.unwrap();

    assert_eq!(session.server().product, "SQLite");
    assert_eq!(session.list_databases().await.unwrap(), ["main"]);

    let tables = session.list_tables("main", None).await.unwrap();
    let names: Vec<_> = tables.iter().map(|t| (t.name.as_str(), t.kind)).collect();
    assert_eq!(
        names,
        [
            ("notes", TableKind::Table),
            ("recent_notes", TableKind::View)
        ]
    );
    session.close().await;
}

#[tokio::test]
async fn sqlite_open_leaves_journal_mode_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path()).await;
    let session = Session::open(&sqlite_params(&path)).await.unwrap();
    session.close().await;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{path}"))
        .await
        .unwrap();
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(mode, "delete");
    assert!(!Path::new(&format!("{path}-wal")).exists());
    pool.close().await;
}

#[tokio::test]
async fn sqlite_missing_file_is_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");
    let error = Session::open(&sqlite_params(&path.to_string_lossy()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "database");
    assert!(error.to_string().contains("找不到数据库文件"), "{error}");
    assert!(!path.exists());
}

#[tokio::test]
async fn session_manager_reuses_open_session_and_forgets_closed_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path()).await;
    let manager = SessionManager::default();

    let first = manager.open("c1", &sqlite_params(&path)).await.unwrap();
    let second = manager.open("c1", &sqlite_params(&path)).await.unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(manager.is_open("c1").await);

    assert!(manager.close("c1").await);
    assert!(!manager.close("c1").await);
    assert_eq!(manager.get("c1").await.unwrap_err().code(), "not_connected");
}

#[tokio::test]
async fn sqlite_rejects_a_file_that_is_not_a_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "这不是数据库,只是一段文字。\n".repeat(200)).unwrap();
    let error = Session::open(&sqlite_params(&path.to_string_lossy()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "database");
    assert!(
        error.to_string().contains("不是有效的 SQLite 数据库"),
        "{error}"
    );
}
