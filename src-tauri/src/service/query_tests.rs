//! SQL 编辑器命令的 service 测试:临时配置目录 + 真实 SQLite 文件,和前端走同一条路径。

use std::sync::Arc;

use basalt_core::config::{ConnectionDraft, DriverKind, Endpoint};
use basalt_core::query::OutcomeStatus;
use basalt_core::secret::MemorySecretStore;

use super::*;
use crate::service;

struct Harness {
    _dir: tempfile::TempDir,
    state: AppState,
    id: String,
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let state = AppState::new(
        dir.path().join("config/connections.json"),
        Arc::new(MemorySecretStore::default()),
    );
    let draft = ConnectionDraft {
        id: None,
        name: "笔记库".into(),
        group: None,
        driver: DriverKind::Sqlite,
        endpoint: Endpoint::File {
            path: db.to_string_lossy().into_owned(),
        },
        save_password: false,
        password: None,
    };
    let id = service::save_connection(&state, draft).await.unwrap().id;
    service::open_connection(&state, &id, None).await.unwrap();
    Harness {
        _dir: dir,
        state,
        id,
    }
}

fn main_db() -> QueryScope {
    QueryScope {
        database: Some("main".into()),
        schema: None,
    }
}

#[tokio::test]
async fn executed_statements_land_in_history() {
    let h = harness().await;
    let inserted = run_statement(
        &h.state,
        "tab",
        &h.id,
        &main_db(),
        "INSERT INTO notes (body) VALUES ('a')",
    )
    .await
    .unwrap();
    assert_eq!(inserted.affected_rows, Some(1));
    let failed = run_statement(&h.state, "tab", &h.id, &main_db(), "SELECT * FROM missing")
        .await
        .unwrap();
    assert_eq!(failed.status, OutcomeStatus::Error);

    let history = search_history(&h.state, "", None, 10).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].sql, "SELECT * FROM missing");
    assert_eq!(history[0].status, OutcomeStatus::Error);
    assert_eq!(history[1].connection_name, "笔记库");
    assert_eq!(history[1].database.as_deref(), Some("main"));
    assert_eq!(
        search_history(&h.state, "insert", Some(&h.id), 10)
            .unwrap()
            .len(),
        1
    );
    assert!(
        search_history(&h.state, "", Some("别的连接"), 10)
            .unwrap()
            .is_empty()
    );

    clear_history(&h.state).unwrap();
    assert!(search_history(&h.state, "", None, 10).unwrap().is_empty());
}

#[tokio::test]
async fn closed_connections_are_reported_not_recorded() {
    let h = harness().await;
    service::close_connection(&h.state, &h.id).await;
    let error = run_statement(&h.state, "tab", &h.id, &main_db(), "SELECT 1")
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_connected");
    assert!(search_history(&h.state, "", None, 10).unwrap().is_empty());
}

#[tokio::test]
async fn cancel_and_close_without_a_running_statement() {
    let h = harness().await;
    assert!(!cancel_statement(&h.state, "tab").await.unwrap());
    run_statement(&h.state, "tab", &h.id, &main_db(), "SELECT 1")
        .await
        .unwrap();
    close_editor(&h.state, "tab").await;
    assert!(!cancel_statement(&h.state, "tab").await.unwrap());
}

#[tokio::test]
async fn columns_for_completion_reuse_the_table_metadata() {
    let h = harness().await;
    let table = TableRef {
        database: "main".into(),
        schema: None,
        table: "notes".into(),
    };
    let columns = list_columns(&h.state, &h.id, &table).await.unwrap();
    let names: Vec<_> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "body"]);
}

#[tokio::test]
async fn saved_queries_round_trip() {
    let h = harness().await;
    let query = SavedQueryRef {
        connection_id: h.id.clone(),
        database: "main".into(),
        schema: None,
        name: "最近的笔记".into(),
    };
    save_query(&h.state, &query, "SELECT * FROM notes;", false).unwrap();
    let error = save_query(&h.state, &query, "SELECT 1", false).unwrap_err();
    assert_eq!(error.code, "already_exists");
    save_query(&h.state, &query, "SELECT 2", true).unwrap();
    let listed = list_saved_queries(&h.state, &h.id, "main", None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "最近的笔记");
    assert_eq!(read_saved_query(&h.state, &query).unwrap(), "SELECT 2");
    delete_saved_query(&h.state, &query).unwrap();
    assert!(
        list_saved_queries(&h.state, &h.id, "main", None)
            .unwrap()
            .is_empty()
    );
    let bad = SavedQueryRef {
        name: "a/b".into(),
        ..query
    };
    assert_eq!(
        save_query(&h.state, &bad, "", false).unwrap_err().code,
        "invalid_name"
    );
}
