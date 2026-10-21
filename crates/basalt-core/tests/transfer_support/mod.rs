//! 导入导出集成测试共用:导出一张表、导回另一张表、逐行比较。
#![allow(dead_code)]

pub mod rss;

use std::path::Path;

use basalt_core::session::SessionManager;
use basalt_core::table::{PageRequest, TableRef};
use basalt_core::transfer::export::{ExportRequest, ExportSource, run_export};
use basalt_core::transfer::import::{
    ImportOptions, ImportReport, ImportRequest, OnError, preview_import, run_import,
};
use basalt_core::transfer::progress::{CancelFlag, ignore};
use basalt_core::transfer::source::SourceSpec;
use basalt_core::transfer::{BinaryEncoding, CsvOptions, ExportFormat, ExportOptions};
use basalt_core::value::CellValue;

pub const CONNECTION: &str = "c1";

pub fn formats(table: &str) -> [(&'static str, ExportFormat); 4] {
    [
        ("csv", ExportFormat::Csv(CsvOptions::default())),
        ("xlsx", ExportFormat::Xlsx),
        ("json", ExportFormat::Json { lines: false }),
        (
            "sql",
            ExportFormat::Sql {
                table: table.to_owned(),
                rows_per_statement: 2,
            },
        ),
    ]
}

pub async fn export_table(
    sessions: &SessionManager,
    table: &TableRef,
    format: ExportFormat,
    path: &Path,
) -> u64 {
    let request = ExportRequest {
        connection_id: CONNECTION.into(),
        source: ExportSource::Table {
            table: table.clone(),
            filter: None,
            sort: None,
        },
        options: ExportOptions {
            format,
            binary: BinaryEncoding::Hex,
        },
        path: path.to_string_lossy().into_owned(),
    };
    run_export(
        sessions,
        "export",
        &request,
        &CancelFlag::default(),
        &ignore,
    )
    .await
    .unwrap_or_else(|e| panic!("导出 {} 失败：{e}", path.display()))
    .rows
}

/// 预览拿到探测好的格式与自动映射,清空目标表后遇错即停地导入(每批 2 行,多走几次批与批之间)。
pub async fn import_file(sessions: &SessionManager, table: &TableRef, path: &Path) -> ImportReport {
    import_file_in_batches(sessions, table, path, 2).await
}

pub async fn import_file_in_batches(
    sessions: &SessionManager,
    table: &TableRef,
    path: &Path,
    batch_size: u32,
) -> ImportReport {
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(sessions, CONNECTION, table, spec.clone())
        .await
        .unwrap();
    let request = ImportRequest {
        connection_id: CONNECTION.into(),
        table: table.clone(),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options: ImportOptions {
            on_error: OnError::Stop,
            batch_size,
            clear_table: true,
            binary: BinaryEncoding::Hex,
        },
    };
    run_import(
        sessions,
        "import",
        &request,
        &CancelFlag::default(),
        &ignore,
    )
    .await
    .unwrap()
}

/// PostgreSQL 的测试库不存在就建上。
pub async fn ensure_postgres_database(admin_url: &str, database: &str) {
    let pool = sqlx::PgPool::connect(admin_url).await.unwrap();
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(database)
            .fetch_one(&pool)
            .await
            .unwrap();
    if !exists {
        sqlx::raw_sql(&format!("CREATE DATABASE {database}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;
}

/// 按主键顺序读出整张表(表数据浏览同一条路径)。
pub async fn rows(sessions: &SessionManager, table: &TableRef) -> Vec<Vec<CellValue>> {
    let session = sessions.get(CONNECTION).await.unwrap();
    let request = PageRequest {
        offset: 0,
        limit: 5000,
        sort: None,
        filter: None,
    };
    session.read_table(table, &request).await.unwrap().rows
}

/// 按序列化的样子比较:浮点数是文本,NaN 和 NaN 相等。
pub fn comparable(row: &[CellValue]) -> serde_json::Value {
    serde_json::to_value(row).unwrap()
}

/// 导出成四种格式,再把 CSV、Excel 导回 `copy`,每次都和原表逐格比较。
pub async fn round_trip(
    sessions: &SessionManager,
    source: &TableRef,
    copy: &TableRef,
    dir: &Path,
    label: &str,
) {
    let expected = rows(sessions, source).await;
    assert!(!expected.is_empty(), "{label}：原表没有数据");
    for (extension, format) in formats(&copy.table) {
        let path = dir.join(format!("{label}.{extension}"));
        let exported = export_table(sessions, source, format, &path).await;
        assert_eq!(exported as usize, expected.len(), "{label} {extension}");
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        if extension == "json" {
            let parsed: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(
                parsed.as_array().unwrap().len(),
                expected.len(),
                "{label} json"
            );
        }
        if extension != "csv" && extension != "xlsx" {
            continue;
        }
        let report = import_file(sessions, copy, &path).await;
        assert_eq!(
            (report.failed, report.inserted as usize),
            (0, expected.len()),
            "{label} {extension}：{}\n{:#?}",
            report.summary,
            report.failures
        );
        let actual = rows(sessions, copy).await;
        assert_eq!(actual.len(), expected.len(), "{label} {extension}");
        for (index, (want, got)) in expected.iter().zip(&actual).enumerate() {
            assert_eq!(
                comparable(want),
                comparable(got),
                "{label} {extension} 第 {} 行往返后不一致",
                index + 1
            );
        }
    }
}
