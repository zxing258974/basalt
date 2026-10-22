//! 导入导出命令的 service 测试:临时配置目录 + 真实 SQLite 文件,和前端走同一条路径。

use basalt_core::config::SslSettings;
use std::sync::{Arc, Mutex};

use basalt_core::config::{ConnectionDraft, DriverKind, Endpoint};
use basalt_core::secret::MemorySecretStore;
use basalt_core::transfer::import::{ImportMode, ImportOptions, ImportStatus, OnError};
use basalt_core::transfer::progress::{Phase, Progress};
use basalt_core::transfer::{BinaryEncoding, CsvOptions, ExportFormat, ExportOptions};

use super::*;
use crate::service;

struct Harness {
    dir: tempfile::TempDir,
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
    sqlx::raw_sql(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
         CREATE TABLE notes_copy (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
         INSERT INTO notes VALUES (1, '第一条'), (2, '');",
    )
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
        ssh: None,
        ssl: SslSettings::default(),
        ssh_secret: None,
    };
    let id = service::save_connection(&state, draft).await.unwrap().id;
    service::open_connection(&state, &id, None, None)
        .await
        .unwrap();
    Harness { dir, state, id }
}

fn table(name: &str) -> TableRef {
    TableRef {
        database: "main".into(),
        schema: None,
        table: name.into(),
    }
}

#[tokio::test]
async fn export_then_import_through_the_service_and_report_progress() {
    let h = harness().await;
    let path = h.dir.path().join("notes.csv");
    let seen = Mutex::new(Vec::new());
    let progress = |p: &Progress| seen.lock().unwrap().push(p.phase);
    let request = ExportRequest {
        connection_id: h.id.clone(),
        source: ExportSource::Table {
            table: table("notes"),
            filter: None,
            sort: None,
        },
        options: ExportOptions {
            format: ExportFormat::Csv(CsvOptions::default()),
            binary: BinaryEncoding::Hex,
        },
        path: path.to_string_lossy().into_owned(),
    };
    let outcome = export_data(&h.state, "job-1", &request, &progress)
        .await
        .unwrap();
    assert_eq!(outcome.rows, 2);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "id,body\r\n1,第一条\r\n2,\"\"\r\n"
    );
    assert!(seen.lock().unwrap().contains(&Phase::Finishing));
    assert!(
        !cancel_transfer(&h.state, "job-1").await.unwrap(),
        "结束的任务不再登记"
    );

    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import_file(&h.state, &h.id, &table("notes_copy"), spec.clone())
        .await
        .unwrap();
    let request = ImportRequest {
        connection_id: h.id.clone(),
        table: table("notes_copy"),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options: ImportOptions {
            on_error: OnError::Stop,
            batch_size: 100,
            mode: ImportMode::Append,
            key_columns: Vec::new(),
            binary: BinaryEncoding::Hex,
        },
    };
    let report = import_data(&h.state, "job-2", &request, &|_: &Progress| {})
        .await
        .unwrap();
    assert_eq!(
        (report.status, report.affected),
        (ImportStatus::Completed, 2)
    );
}

#[tokio::test]
async fn dump_goes_through_the_job_registry() {
    let h = harness().await;
    let path = h.dir.path().join("notes.sql");
    let request = DumpRequest {
        connection_id: h.id.clone(),
        table: table("notes"),
        with_data: true,
        path: path.to_string_lossy().into_owned(),
    };
    let seen = Mutex::new(Vec::new());
    let progress = |p: &Progress| seen.lock().unwrap().push(p.phase);
    let outcome = dump_table(&h.state, "dump-1", &request, &progress)
        .await
        .unwrap();
    assert_eq!(outcome.rows, 2);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("CREATE TABLE notes"), "{text}");
    assert!(text.contains("(1, '第一条'),\n  (2, '');"), "{text}");
    assert!(seen.lock().unwrap().contains(&Phase::Finishing));
    assert!(!cancel_transfer(&h.state, "dump-1").await.unwrap());
}

#[tokio::test]
async fn jobs_can_be_cancelled_by_id_and_ids_are_unique() {
    let h = harness().await;
    let cancel = h
        .state
        .transfers
        .start("job", "transfer:job".into())
        .unwrap();
    let again = h
        .state
        .transfers
        .start("job", "transfer:job".into())
        .unwrap_err();
    assert_eq!(again.code, "already_exists");
    assert!(cancel_transfer(&h.state, "job").await.unwrap());
    assert!(cancel.is_cancelled());
    h.state.transfers.finish("job");
    assert!(!cancel_transfer(&h.state, "job").await.unwrap());

    // 开始之前就取消了:导出不会动手,也不留下文件。
    let path = h.dir.path().join("never.csv");
    let request = ExportRequest {
        connection_id: h.id.clone(),
        source: ExportSource::Table {
            table: table("notes"),
            filter: None,
            sort: None,
        },
        options: ExportOptions {
            format: ExportFormat::Xlsx,
            binary: BinaryEncoding::Hex,
        },
        path: path.to_string_lossy().into_owned(),
    };
    let busy = h.state.transfers.start("busy", "x".into()).unwrap();
    busy.cancel();
    let error = export_data(&h.state, "busy", &request, &|_: &Progress| {})
        .await
        .unwrap_err();
    assert_eq!(error.code, "already_exists", "同一个 id 的任务还没结束");
    assert!(!path.exists());
}

#[tokio::test]
async fn failure_reports_are_saved_and_reveal_checks_the_path() {
    let h = harness().await;
    let path = h.dir.path().join("failures.csv");
    let size = save_import_failures(
        &path.to_string_lossy(),
        &[ImportFailure {
            row: 2,
            column: Some("body".into()),
            value: None,
            reason: "这一列不能为空".into(),
        }],
    )
    .unwrap();
    assert!(size > 0);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("2,body,,这一列不能为空")
    );
    assert_eq!(
        reveal_in_finder("relative.csv").unwrap_err().code,
        "not_found"
    );
    let missing = h.dir.path().join("missing.csv");
    assert_eq!(
        reveal_in_finder(&missing.to_string_lossy())
            .unwrap_err()
            .code,
        "not_found"
    );
}
