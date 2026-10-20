use super::*;
use crate::transfer::progress::ignore;
use crate::transfer::test_support::{SQLITE_SCHEMA, open_sqlite, read_text, table_ref};
use crate::transfer::{BinaryEncoding, CsvOptions, TextEncoding};

fn csv() -> ExportOptions {
    ExportOptions {
        format: ExportFormat::Csv(CsvOptions::default()),
        binary: BinaryEncoding::Hex,
    }
}

fn table_request(
    dir: &std::path::Path,
    name: &str,
    options: ExportOptions,
    filter: Option<&str>,
) -> ExportRequest {
    ExportRequest {
        connection_id: "c1".into(),
        source: ExportSource::Table {
            table: table_ref("people"),
            filter: filter.map(str::to_owned),
            sort: None,
        },
        options,
        path: dir.join(name).to_string_lossy().into_owned(),
    }
}

#[tokio::test]
async fn exports_a_whole_table_to_csv_keeping_every_value() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let request = table_request(dir.path(), "people.csv", csv(), None);
    let outcome = run_export(&sessions, "job", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap();
    assert_eq!(outcome.rows, 3);
    let text = read_text(&request.path);
    assert_eq!(outcome.bytes, text.len() as u64);
    assert_eq!(
        text,
        "id,name,note,big,ratio,avatar\r\n\
         1,张三,,9007199254740993,0.1,0x00ff\r\n\
         2,\"O'Brien, Jr.\",\"\",-5,,\r\n\
         3,李四,\"两\n行\",0,1e300,0x\r\n"
    );
    assert!(
        !sessions.is_open("transfer:job").await,
        "导出用的连接 id 不是连接"
    );
}

#[tokio::test]
async fn filter_and_sort_follow_the_table_page() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let mut request = table_request(
        dir.path(),
        "f.json",
        ExportOptions {
            format: ExportFormat::Json { lines: true },
            binary: BinaryEncoding::Base64,
        },
        Some("id >= 2"),
    );
    if let ExportSource::Table { sort, .. } = &mut request.source {
        *sort = Some(SortSpec {
            column: "id".into(),
            descending: true,
        });
    }
    let outcome = run_export(&sessions, "j", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap();
    assert_eq!(outcome.rows, 2);
    let text = read_text(&request.path);
    let ids: Vec<i64> = text
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["id"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(ids, [3, 2]);
    assert!(
        text.contains("\"avatar\":\"\""),
        "空字节串的 Base64 是空字符串：{text}"
    );
}

#[tokio::test]
async fn query_results_rerun_on_the_tab_connection_or_use_shown_rows() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let path = dir.path().join("q.sql").to_string_lossy().into_owned();
    let options = ExportOptions {
        format: ExportFormat::Sql {
            table: "people_copy".into(),
            rows_per_statement: 2,
        },
        binary: BinaryEncoding::Hex,
    };
    let request = ExportRequest {
        connection_id: "c1".into(),
        source: ExportSource::Query {
            editor_id: "tab-1".into(),
            scope: QueryScope::default(),
            sql: "SELECT id, name FROM people WHERE id < 3 ORDER BY id".into(),
        },
        options: options.clone(),
        path: path.clone(),
    };
    let outcome = run_export(&sessions, "j", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap();
    assert_eq!(outcome.rows, 2);
    assert!(
        read_text(&path).ends_with(
            "INSERT INTO \"people_copy\" (\"id\", \"name\") VALUES\n  (1, '张三'),\n  (2, 'O''Brien, Jr.');\n"
        ),
        "{}",
        read_text(&path)
    );

    // 没有行:列名照样写出来。
    let empty = ExportRequest {
        source: ExportSource::Query {
            editor_id: "tab-1".into(),
            scope: QueryScope::default(),
            sql: "SELECT id, name FROM people WHERE 0".into(),
        },
        options: csv(),
        ..request.clone()
    };
    run_export(&sessions, "j", &empty, &CancelFlag::default(), &ignore)
        .await
        .unwrap();
    assert_eq!(read_text(&path), "id,name\r\n");

    let rows = ExportRequest {
        source: ExportSource::Rows {
            columns: vec![ResultColumn {
                name: "n".into(),
                data_type: String::new(),
            }],
            rows: vec![vec![CellValue::Int(7)], vec![CellValue::Null]],
        },
        options: csv(),
        ..request.clone()
    };
    let outcome = run_export(&sessions, "j", &rows, &CancelFlag::default(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        (outcome.rows, read_text(&path)),
        (2, "n\r\n7\r\n\r\n".into())
    );

    let writes = ExportRequest {
        source: ExportSource::Query {
            editor_id: "tab-1".into(),
            scope: QueryScope::default(),
            sql: "DELETE FROM people RETURNING *".into(),
        },
        ..request
    };
    let error = run_export(&sessions, "j", &writes, &CancelFlag::default(), &ignore)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_data");
}

#[tokio::test]
async fn cancelling_or_failing_leaves_no_file_behind() {
    let (dir, sessions) = open_sqlite(SQLITE_SCHEMA).await;
    let request = table_request(dir.path(), "c.csv", csv(), None);
    let cancel = CancelFlag::default();
    // 读到第一行时取消。
    let stop = cancel.clone();
    let progress = move |p: &Progress| {
        if p.phase == Phase::Exporting {
            stop.cancel();
        }
    };
    let error = run_export(&sessions, "j", &request, &cancel, &progress)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "cancelled");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "只剩数据库文件"
    );

    let gbk = ExportOptions {
        format: ExportFormat::Csv(CsvOptions {
            encoding: TextEncoding::Gbk,
            ..CsvOptions::default()
        }),
        binary: BinaryEncoding::Hex,
    };
    sqlx_exec(&sessions, "UPDATE people SET name = '笑😀' WHERE id = 2").await;
    let request = table_request(dir.path(), "g.csv", gbk, None);
    let error = run_export(&sessions, "j", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("第 2 行「name」"), "{error}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

    // 连接还能接着用。
    let request = table_request(dir.path(), "ok.csv", csv(), None);
    run_export(&sessions, "j", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap();
}

async fn sqlx_exec(sessions: &SessionManager, sql: &str) {
    let outcome = sessions
        .run_query("setup", "c1", &QueryScope::default(), sql)
        .await
        .unwrap();
    assert_eq!(
        outcome.status,
        crate::query::OutcomeStatus::Ok,
        "{:?}",
        outcome.message
    );
}

#[test]
fn only_read_statements_can_be_run_again() {
    for sql in [
        "SELECT * FROM t",
        "  -- 说明\n with x as (select 1) select * from x",
        "SHOW TABLES",
        "select 'insert into' as label",
        "SELECT `update` FROM t /* delete */",
        "VALUES (1), (2)",
    ] {
        assert!(is_rerunnable(sql), "{sql}");
    }
    for sql in [
        "INSERT INTO t VALUES (1)",
        "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
        "SELECT * INTO backup FROM t",
        "CALL refresh()",
        "EXPLAIN ANALYZE DELETE FROM t",
        "UPDATE t SET a = 1 RETURNING *",
        "",
    ] {
        assert!(!is_rerunnable(sql), "{sql}");
    }
}
