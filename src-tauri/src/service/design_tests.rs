//! 表结构设计命令的 service 层测试:与前端走同一条路径(保存连接 → 打开 → 预览 → 执行 →
//! 再读)。SQLite 总会运行;BASALT_IT 选中时再连 MySQL 8.4 与 PostgreSQL 17。

use basalt_core::config::SslSettings;
use std::sync::Arc;

use basalt_core::config::{ConnectionDraft, DriverKind, Endpoint};
use basalt_core::design::{ColumnDef, ExecMode, TableDesign};
use basalt_core::secret::MemorySecretStore;
use basalt_core::table::TableRef;

use crate::service;
use crate::state::AppState;

/// 本机一次性测试容器的密码,与 docker-compose.test.yml 一致。
const TEST_PASSWORD: &str = "basalt-test";

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

fn column(name: &str, data_type: &str) -> ColumnDef {
    ColumnDef {
        id: format!("new:{name}"),
        name: name.into(),
        data_type: data_type.into(),
        nullable: name != "id",
        default: None,
        auto_increment: false,
        comment: None,
        collation: None,
        on_update: None,
        generated: None,
    }
}

fn design(table: TableRef, id_type: &str) -> TableDesign {
    TableDesign {
        version: String::new(),
        table,
        columns: vec![column("id", id_type), column("body", "text")],
        primary_key: vec!["new:id".into()],
        primary_key_name: None,
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
        triggers: Vec::new(),
        options: Default::default(),
    }
}

/// 建表 → 读回 → 字段改名并加字段 → 读回,断言预览的执行方式。
async fn create_and_rename(
    state: &AppState,
    id: &str,
    table: TableRef,
    id_type: &str,
    mode: ExecMode,
) {
    let target = design(table.clone(), id_type);
    let preview = service::preview_table_design(state, id, None, &target)
        .await
        .unwrap();
    assert_eq!(preview.mode, mode);
    let outcome = service::apply_table_design(state, id, None, &target, &preview.statements)
        .await
        .unwrap();
    assert_eq!(outcome.table, table);

    let mut next = service::read_table_design(state, id, &table).await.unwrap();
    next.columns[1].name = "content".into();
    next.columns.push(column("extra", "text"));
    let preview = service::preview_table_design(state, id, Some(&table), &next)
        .await
        .unwrap();
    assert!(
        preview
            .statements
            .iter()
            .all(|s| !s.contains("DROP COLUMN")),
        "改名不能变成删了再加:{:?}",
        preview.statements
    );
    service::apply_table_design(state, id, Some(&table), &next, &preview.statements)
        .await
        .unwrap();
    let read = service::read_table_design(state, id, &table).await.unwrap();
    let names: Vec<_> = read.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "content", "extra"]);

    let error = service::apply_table_design(state, id, Some(&table), &next, &["旧预览".into()])
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_edit");
}

#[tokio::test]
async fn sqlite_design_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("svc.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    sqlx::SqlitePool::connect_with(options)
        .await
        .unwrap()
        .close()
        .await;

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
        ssh: None,
        ssl: SslSettings::default(),
        ssh_secret: None,
    };
    let saved = service::save_connection(&state, draft).await.unwrap();
    let table = TableRef {
        database: "main".into(),
        schema: None,
        table: "notes".into(),
    };
    let closed = service::read_table_design(&state, &saved.id, &table)
        .await
        .unwrap_err();
    assert_eq!(closed.code, "not_connected");

    service::open_connection(&state, &saved.id, None, None)
        .await
        .unwrap();
    create_and_rename(&state, &saved.id, table, "INTEGER", ExecMode::Transaction).await;
}

async fn network(state: &AppState, driver: DriverKind, port: u16, username: &str) -> String {
    let draft = ConnectionDraft {
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
        ssh: None,
        ssl: SslSettings::default(),
        ssh_secret: None,
    };
    let saved = service::save_connection(state, draft).await.unwrap();
    service::open_connection(state, &saved.id, None, None)
        .await
        .unwrap();
    saved.id
}

#[tokio::test]
async fn mysql84_design_round_trip() {
    if !enabled("mysql84") {
        return;
    }
    let pool = sqlx::MySqlPool::connect(&format!("mysql://root:{TEST_PASSWORD}@127.0.0.1:23307/"))
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE DATABASE IF NOT EXISTS basalt_svc; DROP TABLE IF EXISTS basalt_svc.m5_notes;",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let id = network(&state, DriverKind::Mysql, 23307, "root").await;
    let table = TableRef {
        database: "basalt_svc".into(),
        schema: None,
        table: "m5_notes".into(),
    };
    create_and_rename(&state, &id, table, "int", ExecMode::Autocommit).await;
}

#[tokio::test]
async fn postgres_design_round_trip() {
    if !enabled("postgres") {
        return;
    }
    let pool = sqlx::PgPool::connect(&format!(
        "postgres://postgres:{TEST_PASSWORD}@127.0.0.1:25432/postgres"
    ))
    .await
    .unwrap();
    sqlx::raw_sql("DROP TABLE IF EXISTS public.m5_notes")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let id = network(&state, DriverKind::Postgres, 25432, "postgres").await;
    let table = TableRef {
        database: "postgres".into(),
        schema: Some("public".into()),
        table: "m5_notes".into(),
    };
    create_and_rename(&state, &id, table, "integer", ExecMode::Transaction).await;
}
