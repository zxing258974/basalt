//! 表数据命令的 service 层测试:与前端走同一条路径(保存连接 → 打开 → 读一页 →
//! 预览 → 提交 → 再读)。SQLite 总会运行;BASALT_IT 选中时再连 MySQL 8.4 与 PostgreSQL 17。

use std::collections::BTreeMap;
use std::sync::Arc;

use basalt_core::config::{ConnectionDraft, DriverKind, Endpoint};
use basalt_core::secret::MemorySecretStore;
use basalt_core::table::{PageRequest, RowChange, SortSpec, TableRef};
use basalt_core::value::CellValue;

use crate::service;
use crate::state::AppState;

/// 本机一次性测试容器的密码,与 docker-compose.test.yml 一致。
const TEST_PASSWORD: &str = "basalt-test";
const FIXTURE_DB: &str = "basalt_svc";

fn enabled(target: &str) -> bool {
    let Ok(raw) = std::env::var("BASALT_IT") else {
        return false;
    };
    match raw.trim() {
        "all" => true,
        "1" => matches!(target, "mysql57" | "postgres"),
        list => list.split(',').any(|t| t.trim() == target),
    }
}

fn state(dir: &std::path::Path) -> AppState {
    AppState::new(
        dir.join("connections.json"),
        Arc::new(MemorySecretStore::default()),
    )
}

fn network_draft(driver: DriverKind, port: u16, username: &str) -> ConnectionDraft {
    ConnectionDraft {
        id: None,
        name: format!("{driver:?} 测试"),
        group: None,
        driver,
        endpoint: Endpoint::Network {
            host: "127.0.0.1".into(),
            port,
            username: username.into(),
            database: None,
        },
        save_password: true,
        password: Some(TEST_PASSWORD.into()),
    }
}

fn first_page() -> PageRequest {
    PageRequest {
        offset: 0,
        limit: 100,
        sort: None,
        filter: None,
    }
}

/// 把 `notes` 表里 id = 1 那行的 body 改成「已改」,断言预览与结果。
async fn edit_one_row(state: &AppState, id: &str, table: &TableRef, expected_preview: &str) {
    let page = service::read_table_page(state, id, table, &first_page())
        .await
        .unwrap();
    assert_eq!(page.read_only_reason, None);
    assert_eq!(
        page.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "body"]
    );
    let key = BTreeMap::from([("id".to_owned(), page.rows[0][0].clone())]);
    let changes = [RowChange::Update {
        key,
        values: BTreeMap::from([("body".to_owned(), Some("已改".to_owned()))]),
    }];

    let preview = service::preview_table_changes(state, id, table, &changes)
        .await
        .unwrap();
    assert_eq!(preview, [expected_preview]);
    let outcome = service::apply_table_changes(state, id, table, &changes, &preview)
        .await
        .unwrap();
    assert_eq!((outcome.statements, outcome.affected_rows), (1, 1));

    let page = service::read_table_page(state, id, table, &first_page())
        .await
        .unwrap();
    assert_eq!(page.rows[0][1], CellValue::Text("已改".into()));
    assert_eq!(
        page.rows[1][1],
        CellValue::Text("第二行".into()),
        "其他行不受影响"
    );
}

#[tokio::test]
async fn sqlite_row_edit_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("svc.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
         INSERT INTO notes VALUES (1, '第一行'), (2, '第二行');",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let state = state(dir.path());
    let draft = ConnectionDraft {
        id: None,
        name: "本地".into(),
        group: None,
        driver: DriverKind::Sqlite,
        endpoint: Endpoint::File {
            path: path.to_string_lossy().into_owned(),
        },
        save_password: false,
        password: None,
    };
    let saved = service::save_connection(&state, draft).await.unwrap();
    service::open_connection(&state, &saved.id, None)
        .await
        .unwrap();
    let table = TableRef {
        database: "main".into(),
        schema: None,
        table: "notes".into(),
    };
    edit_one_row(
        &state,
        &saved.id,
        &table,
        "UPDATE \"main\".\"notes\" SET \"body\" = '已改' WHERE \"id\" = 1",
    )
    .await;

    // 命令层的错误码透传给前端。
    let bad_sort = PageRequest {
        sort: Some(SortSpec {
            column: "nope".into(),
            descending: false,
        }),
        ..first_page()
    };
    let error = service::read_table_page(&state, &saved.id, &table, &bad_sort)
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_filter");
    service::close_connection(&state, &saved.id).await;
    let error = service::read_table_page(&state, &saved.id, &table, &first_page())
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_connected");
}

#[tokio::test]
async fn mysql84_row_edit_round_trip() {
    if !enabled("mysql84") {
        return;
    }
    let url = format!("mysql://root:{TEST_PASSWORD}@127.0.0.1:23307/");
    let pool = sqlx::MySqlPool::connect(&url)
        .await
        .expect("连接 MySQL 8.4 测试容器");
    sqlx::raw_sql(
        "CREATE DATABASE IF NOT EXISTS basalt_svc CHARACTER SET utf8mb4;
         DROP TABLE IF EXISTS basalt_svc.notes;
         CREATE TABLE basalt_svc.notes (id INT PRIMARY KEY, body VARCHAR(50));
         INSERT INTO basalt_svc.notes VALUES (1, '第一行'), (2, '第二行');",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let saved = service::save_connection(&state, network_draft(DriverKind::Mysql, 23307, "root"))
        .await
        .unwrap();
    let server = service::open_connection(&state, &saved.id, None)
        .await
        .unwrap();
    assert!(server.version.starts_with("8.4"), "{server:?}");
    let table = TableRef {
        database: FIXTURE_DB.into(),
        schema: None,
        table: "notes".into(),
    };
    edit_one_row(
        &state,
        &saved.id,
        &table,
        "UPDATE `basalt_svc`.`notes` SET `body` = '已改' WHERE `id` = 1",
    )
    .await;
    service::close_connection(&state, &saved.id).await;
}

#[tokio::test]
async fn postgres17_row_edit_round_trip() {
    if !enabled("postgres") {
        return;
    }
    let base = format!("postgres://postgres:{TEST_PASSWORD}@127.0.0.1:25432");
    let admin = sqlx::PgPool::connect(&format!("{base}/postgres"))
        .await
        .expect("连接 PG 测试容器");
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
        .bind(FIXTURE_DB)
        .fetch_optional(&admin)
        .await
        .unwrap();
    if exists.is_none() {
        sqlx::raw_sql("CREATE DATABASE basalt_svc")
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
    let db = sqlx::PgPool::connect(&format!("{base}/{FIXTURE_DB}"))
        .await
        .unwrap();
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS public.notes;
         CREATE TABLE public.notes (id integer PRIMARY KEY, body varchar(50));
         INSERT INTO public.notes VALUES (1, '第一行'), (2, '第二行');",
    )
    .execute(&db)
    .await
    .unwrap();
    db.close().await;

    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let saved = service::save_connection(
        &state,
        network_draft(DriverKind::Postgres, 25432, "postgres"),
    )
    .await
    .unwrap();
    let server = service::open_connection(&state, &saved.id, None)
        .await
        .unwrap();
    assert!(server.version.starts_with("17"), "{server:?}");
    let table = TableRef {
        database: FIXTURE_DB.into(),
        schema: Some("public".into()),
        table: "notes".into(),
    };
    edit_one_row(
        &state,
        &saved.id,
        &table,
        "UPDATE \"public\".\"notes\" SET \"body\" = '已改' WHERE \"id\" = '1'::integer",
    )
    .await;
    service::close_connection(&state, &saved.id).await;
}
