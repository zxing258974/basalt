use super::*;
use crate::query::{ResultColumn, ResultSet};
use crate::value::CellValue;

fn outcome(status: OutcomeStatus) -> StatementOutcome {
    StatementOutcome {
        status,
        message: None,
        result_sets: Vec::new(),
        affected_rows: None,
        elapsed_ms: 7,
        scope: QueryScope::default(),
        notice: None,
        messages: Vec::new(),
        error_position: None,
    }
}

fn entry(connection: &str, sql: &str) -> HistoryEntry {
    let scope = QueryScope {
        database: Some("shop".into()),
        schema: None,
    };
    HistoryEntry::record(
        connection,
        "本地 MySQL",
        &scope,
        sql,
        &outcome(OutcomeStatus::Ok),
    )
}

fn store(dir: &tempfile::TempDir, capacity: usize) -> HistoryStore {
    HistoryStore::with_capacity(dir.path().join("sub").join("history.jsonl"), capacity)
}

#[test]
fn record_counts_rows_and_keeps_scope() {
    let mut done = outcome(OutcomeStatus::Ok);
    done.result_sets = vec![
        ResultSet {
            id: 1,
            columns: vec![ResultColumn {
                name: "a".into(),
                data_type: String::new(),
                source: None,
            }],
            rows: vec![vec![CellValue::Int(1)]],
            total_rows: 2,
            truncated: false,
            read_only: None,
        },
        ResultSet {
            id: 2,
            columns: Vec::new(),
            rows: vec![Vec::new()],
            total_rows: 1,
            truncated: false,
            read_only: None,
        },
    ];
    let scope = QueryScope {
        database: Some("folio".into()),
        schema: Some("public".into()),
    };
    let recorded = HistoryEntry::record("c1", "PG", &scope, "SELECT 1", &done);
    assert_eq!(recorded.row_count, Some(3));
    assert_eq!(recorded.database.as_deref(), Some("folio"));
    assert_eq!(recorded.schema.as_deref(), Some("public"));
    assert_eq!(recorded.elapsed_ms, 7);
    assert!(recorded.executed_at.contains('T'), "RFC 3339");
    assert_eq!(
        HistoryEntry::record(
            "c1",
            "PG",
            &scope,
            "UPDATE t",
            &outcome(OutcomeStatus::Error)
        )
        .row_count,
        None
    );
}

#[test]
fn very_long_sql_is_cut() {
    let sql = "x".repeat(MAX_SQL_CHARS + 10);
    assert_eq!(entry("c1", &sql).sql.chars().count(), MAX_SQL_CHARS);
}

#[test]
fn missing_file_searches_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(store(&dir, 10).search("", None, 10).unwrap().is_empty());
}

#[test]
fn search_returns_newest_first_and_filters() {
    let dir = tempfile::tempdir().unwrap();
    let history = store(&dir, 10);
    history
        .append(&entry("c1", "SELECT * FROM orders"))
        .unwrap();
    history
        .append(&entry("c2", "select id FROM Customers"))
        .unwrap();
    history
        .append(&entry("c1", "DELETE FROM orders WHERE id = 1"))
        .unwrap();

    let all = history.search("", None, 10).unwrap();
    assert_eq!(
        all.iter().map(|e| e.sql.as_str()).collect::<Vec<_>>(),
        [
            "DELETE FROM orders WHERE id = 1",
            "select id FROM Customers",
            "SELECT * FROM orders"
        ]
    );
    let orders = history.search("ORDERS delete", None, 10).unwrap();
    assert_eq!(orders.len(), 1, "每个词都要命中，不分大小写");
    assert_eq!(
        history.search("customers", Some("c1"), 10).unwrap().len(),
        0
    );
    assert_eq!(
        history.search("本地", Some("c1"), 10).unwrap().len(),
        2,
        "连接名也能搜"
    );
    assert_eq!(history.search("shop", None, 1).unwrap().len(), 1, "limit");
}

#[test]
fn broken_lines_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let history = store(&dir, 10);
    history.append(&entry("c1", "SELECT 1")).unwrap();
    let path = dir.path().join("sub").join("history.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("{\"id\": \"写到一半\n\n");
    std::fs::write(&path, text).unwrap();
    history.append(&entry("c1", "SELECT 2")).unwrap();
    let found = history.search("", None, 10).unwrap();
    assert_eq!(
        found.iter().map(|e| e.sql.as_str()).collect::<Vec<_>>(),
        ["SELECT 2", "SELECT 1"]
    );
}

#[test]
fn keeps_only_the_most_recent_entries() {
    let dir = tempfile::tempdir().unwrap();
    let history = store(&dir, 3);
    for n in 0..(3 + COMPACT_SLACK + 1) {
        history
            .append(&entry("c1", &format!("SELECT {n}")))
            .unwrap();
    }
    let kept = history.search("", None, 1000).unwrap();
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[0].sql, format!("SELECT {}", 3 + COMPACT_SLACK));
    // 整理之后继续追加,计数接着记。
    history.append(&entry("c1", "SELECT last")).unwrap();
    assert_eq!(history.search("", None, 1000).unwrap().len(), 4);
}

#[test]
fn counts_existing_lines_on_first_append() {
    let dir = tempfile::tempdir().unwrap();
    let first = store(&dir, 2);
    for n in 0..(2 + COMPACT_SLACK) {
        first.append(&entry("c1", &format!("SELECT {n}"))).unwrap();
    }
    // 重启应用:新的 store 不知道文件里已有多少行,第一次追加时数出来,超了就整理。
    let reopened = store(&dir, 2);
    reopened.append(&entry("c1", "SELECT new")).unwrap();
    assert_eq!(reopened.search("", None, 1000).unwrap().len(), 2);
}

#[test]
fn clear_removes_everything() {
    let dir = tempfile::tempdir().unwrap();
    let history = store(&dir, 10);
    history.clear().unwrap();
    history.append(&entry("c1", "SELECT 1")).unwrap();
    history.clear().unwrap();
    assert!(history.search("", None, 10).unwrap().is_empty());
    history.append(&entry("c1", "SELECT 2")).unwrap();
    assert_eq!(history.search("", None, 10).unwrap().len(), 1);
}
