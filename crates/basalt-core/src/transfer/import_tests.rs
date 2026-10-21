use super::*;
use crate::query::OutcomeStatus;
use crate::transfer::export::{ExportRequest, ExportSource, run_export};
use crate::transfer::progress::{Progress, ignore};
use crate::transfer::source::SourceFormat;
use crate::transfer::test_support::{SQLITE_SCHEMA, open_sqlite, table_ref};
use crate::transfer::{CsvOptions, ExportFormat, ExportOptions};
use crate::value::CellValue;

fn options(on_error: OnError, batch_size: u32, clear_table: bool) -> ImportOptions {
    ImportOptions {
        on_error,
        batch_size,
        mode: if clear_table {
            ImportMode::Replace
        } else {
            ImportMode::Append
        },
        key_columns: Vec::new(),
        binary: BinaryEncoding::Hex,
    }
}

async fn query(sessions: &SessionManager, sql: &str) -> Vec<Vec<CellValue>> {
    let outcome = sessions
        .run_query("check", "c1", &QueryScope::default(), sql)
        .await
        .unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Ok, "{:?}", outcome.message);
    outcome
        .result_sets
        .into_iter()
        .next()
        .map(|s| s.rows)
        .unwrap_or_default()
}

async fn count(sessions: &SessionManager, table: &str) -> i64 {
    match query(sessions, &format!("SELECT COUNT(*) FROM {table}")).await[0][0] {
        CellValue::Int(n) => n,
        ref other => panic!("{other:?}"),
    }
}

/// 预览一遍拿到探测好的格式和自动映射,再按给定选项导入。
async fn import(
    sessions: &SessionManager,
    path: &std::path::Path,
    table: &str,
    options: ImportOptions,
) -> ImportReport {
    import_with(
        sessions,
        path,
        table,
        options,
        &CancelFlag::default(),
        &ignore,
    )
    .await
}

async fn import_with(
    sessions: &SessionManager,
    path: &std::path::Path,
    table: &str,
    options: ImportOptions,
    cancel: &CancelFlag,
    progress: &dyn ProgressSink,
) -> ImportReport {
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(sessions, "c1", &table_ref(table), spec.clone())
        .await
        .unwrap();
    let request = ImportRequest {
        connection_id: "c1".into(),
        table: table_ref(table),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options,
    };
    run_import(sessions, "job", &request, cancel, progress)
        .await
        .unwrap()
}

const SAME: &str = "SELECT COUNT(*) FROM (SELECT * FROM people EXCEPT SELECT * FROM people_copy
                    UNION ALL SELECT * FROM people_copy EXCEPT SELECT * FROM people)";

#[tokio::test]
async fn csv_and_excel_round_trip_null_chinese_binary_and_big_numbers() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    for (name, format) in [
        ("p.csv", ExportFormat::Csv(CsvOptions::default())),
        ("p.xlsx", ExportFormat::Xlsx),
    ] {
        let path = dir.path().join(name);
        let request = ExportRequest {
            connection_id: "c1".into(),
            source: ExportSource::Table {
                table: table_ref("people"),
                filter: None,
                sort: None,
            },
            options: ExportOptions {
                format,
                binary: BinaryEncoding::Hex,
            },
            path: path.to_string_lossy().into_owned(),
        };
        run_export(&sessions, "e", &request, &CancelFlag::default(), &ignore)
            .await
            .unwrap();
        let report = import(
            &sessions,
            &path,
            "people_copy",
            options(OnError::Stop, 500, true),
        )
        .await;
        assert_eq!(
            report.status,
            ImportStatus::Completed,
            "{name}：{}",
            report.summary
        );
        assert_eq!(report.affected, 3, "{name}");
        assert_eq!(
            query(&sessions, SAME).await,
            [[CellValue::Int(0)]],
            "{name} 往返后不一致"
        );
        let types = query(
            &sessions,
            "SELECT typeof(note), typeof(avatar), big FROM people_copy ORDER BY id",
        )
        .await;
        assert_eq!(
            types[0][0],
            CellValue::Text("null".into()),
            "{name}：NULL 还是 NULL"
        );
        assert_eq!(
            types[1][0],
            CellValue::Text("text".into()),
            "{name}：空字符串不是 NULL"
        );
        assert_eq!(types[0][1], CellValue::Text("blob".into()), "{name}");
        assert_eq!(types[0][2], CellValue::Int(9_007_199_254_740_993), "{name}");
    }
}

fn write(dir: &tempfile::TempDir, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[tokio::test]
async fn skipping_records_bad_values_rejected_rows_and_misaligned_rows() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let path = write(
        &dir,
        "s.csv",
        "id,name,big\n1,甲,10\n2,,x\n3,丙,5\n3,丁,6\n4,戊,1,多出来\n5,己,\n",
    );
    let report = import(
        &sessions,
        &path,
        "people_copy",
        options(OnError::Skip, 500, false),
    )
    .await;
    assert_eq!(report.status, ImportStatus::Completed, "{}", report.summary);
    assert_eq!((report.affected, report.failed), (3, 3));
    let mut failures = report.failures.clone();
    failures.sort_by_key(|f| f.row);
    assert_eq!(failures[0].row, 3);
    assert_eq!(failures[0].column.as_deref(), Some("name"));
    assert!(failures[0].reason.contains("不能为空"));
    assert_eq!(failures[1].row, 5);
    assert_eq!(failures[1].column, None);
    assert!(
        failures[1].reason.contains("UNIQUE"),
        "{}",
        failures[1].reason
    );
    assert_eq!(failures[2].row, 6);
    assert!(
        failures[2].reason.contains("多 1 个"),
        "{}",
        failures[2].reason
    );
    assert!(
        report.summary.contains("成功 3 行") && report.summary.contains("3 行失败"),
        "{}",
        report.summary
    );
    assert_eq!(
        query(&sessions, "SELECT id, big FROM people_copy ORDER BY id").await,
        [
            [CellValue::Int(1), CellValue::Int(10)],
            [CellValue::Int(3), CellValue::Int(5)],
            [CellValue::Int(5), CellValue::Null]
        ]
    );
}

#[tokio::test]
async fn stopping_checks_the_whole_file_before_writing_anything() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let path = write(
        &dir,
        "c.csv",
        "id,name,big\n1,甲,1\n2,乙,2.5\n3,丙,3\n4,丁,四\n",
    );
    let report = import(
        &sessions,
        &path,
        "people_copy",
        options(OnError::Stop, 1, false),
    )
    .await;
    assert_eq!(report.status, ImportStatus::Stopped);
    assert_eq!((report.affected, report.failed), (0, 2));
    assert_eq!(
        report.failures.iter().map(|f| f.row).collect::<Vec<_>>(),
        [3, 5]
    );
    assert_eq!(report.failures[0].value.as_deref(), Some("2.5"));
    assert!(report.summary.contains("什么都没有写入"));
    assert_eq!(count(&sessions, "people_copy").await, 0);
}

#[tokio::test]
async fn a_rejected_row_rolls_back_everything_including_the_clear() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    query(
        &sessions,
        "INSERT INTO people_copy (id, name) VALUES (9, '原有')",
    )
    .await;
    let path = write(&dir, "d.csv", "id,name\n1,甲\n2,乙\n2,重复\n");
    let report = import(
        &sessions,
        &path,
        "people_copy",
        options(OnError::Stop, 1, true),
    )
    .await;
    assert_eq!(report.status, ImportStatus::Stopped);
    assert_eq!(report.affected, 0);
    assert_eq!(report.failures[0].row, 4);
    assert!(report.summary.contains("回滚"), "{}", report.summary);
    assert_eq!(
        query(&sessions, "SELECT id FROM people_copy").await,
        [[CellValue::Int(9)]],
        "清空表也撤回了"
    );
}

#[tokio::test]
async fn cancelling_rolls_back() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let text: String = std::iter::once("id,name\n".to_owned())
        .chain((1..=20).map(|n| format!("{n},名{n}\n")))
        .collect();
    let path = write(&dir, "x.csv", &text);
    let cancel = CancelFlag::default();
    let stop = cancel.clone();
    let progress = move |p: &Progress| {
        if p.phase == Phase::Importing {
            stop.cancel();
        }
    };
    let report = import_with(
        &sessions,
        &path,
        "people_copy",
        options(OnError::Skip, 2, false),
        &cancel,
        &progress,
    )
    .await;
    assert_eq!(report.status, ImportStatus::Cancelled);
    assert_eq!(report.affected, 0);
    assert!(report.summary.contains("已取消"), "{}", report.summary);
    assert_eq!(count(&sessions, "people_copy").await, 0);
}

#[tokio::test]
async fn preview_and_plan_validation() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let path = write(&dir, "m.csv", "名字,ID\n张三,7\n");
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(&sessions, "c1", &table_ref("people_copy"), spec.clone())
        .await
        .unwrap();
    assert_eq!(preview.columns.len(), 6);
    assert_eq!(
        preview.mapping[0],
        ColumnMapping {
            column: "id".into(),
            source: Some(1)
        }
    );
    assert!(matches!(
        preview.source.format,
        SourceFormat::Csv {
            delimiter: Some(','),
            ..
        }
    ));

    let request = ImportRequest {
        connection_id: "c1".into(),
        table: table_ref("people_copy"),
        source: SourceSpec {
            format: Some(preview.source.format.clone()),
            ..spec
        },
        mapping: vec![ColumnMapping {
            column: "id".into(),
            source: None,
        }],
        options: options(OnError::Skip, 10, false),
    };
    let error = run_import(&sessions, "j", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("至少"), "{error}");
    let unknown = ImportRequest {
        mapping: vec![ColumnMapping {
            column: "nope".into(),
            source: Some(0),
        }],
        ..request
    };
    let error = run_import(&sessions, "j", &unknown, &CancelFlag::default(), &ignore)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("nope"), "{error}");
}

#[tokio::test]
async fn all_null_rows_are_kept_and_short_rows_are_reported() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let path = write(&dir, "n.csv", "id,name,big\n,,\n7\n8,庚,1\n\n");
    let report = import(
        &sessions,
        &path,
        "people_copy",
        options(OnError::Skip, 10, false),
    )
    .await;
    assert_eq!(
        (report.affected, report.failed),
        (1, 2),
        "{}",
        report.summary
    );
    let mut failures = report.failures.clone();
    failures.sort_by_key(|f| f.row);
    assert_eq!(failures[0].row, 2, "全是 NULL 的一行不能悄悄丢掉");
    assert!(failures[0].reason.contains("不能为空"));
    assert_eq!(failures[1].row, 3);
    assert!(
        failures[1].reason.contains("少 2 个"),
        "{}",
        failures[1].reason
    );
}
