//! 查询标签页在 SQLite 上执行用户的 SQL:会话状态、结果集、影响行数、取消。不需要容器。

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use basalt_core::query::results::{FIRST_BATCH, ResultSort};
use basalt_core::query::{ColumnSource, OutcomeStatus, QueryScope, StatementOutcome};
use basalt_core::session::SessionManager;
use basalt_core::table::TableRef;
use basalt_core::value::CellValue;
use common::*;

const SCHEMA: &str = "
    CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price REAL, raw BLOB);
    INSERT INTO items VALUES (1, 'pen', 1.5, x'00ff'), (2, '本子', NULL, NULL);
";

/// 打开一个带 items 表的 SQLite 连接,id 是 c1。
async fn manager(dir: &tempfile::TempDir) -> SessionManager {
    let path = sqlite_file(dir.path(), "q.db", SCHEMA).await;
    let manager = SessionManager::default();
    manager.open("c1", &sqlite_params(&path)).await.unwrap();
    manager
}

fn scope() -> QueryScope {
    QueryScope {
        database: Some("main".into()),
        schema: None,
    }
}

async fn run(manager: &SessionManager, editor: &str, sql: &str) -> StatementOutcome {
    manager
        .run_query(editor, "c1", &scope(), sql)
        .await
        .unwrap()
}

#[tokio::test]
async fn statements_in_one_tab_share_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    let created = run(&manager, "tab1", "CREATE TEMP TABLE scratch (x INTEGER)").await;
    assert_eq!(created.status, OutcomeStatus::Ok);
    assert_eq!(created.affected_rows, None, "DDL 没有影响行数");
    let inserted = run(&manager, "tab1", "INSERT INTO scratch VALUES (1), (2), (3)").await;
    assert_eq!(inserted.affected_rows, Some(3));
    let selected = run(&manager, "tab1", "SELECT x FROM scratch ORDER BY x").await;
    assert_eq!(selected.result_sets.len(), 1);
    assert_eq!(selected.result_sets[0].rows.len(), 3);
    assert_eq!(
        selected.affected_rows, None,
        "SQLite 对 SELECT 报的是上一次的变更数，不能信"
    );

    // 临时表只在这个标签页自己的连接上。
    let other = run(&manager, "tab2", "SELECT x FROM scratch").await;
    assert_eq!(other.status, OutcomeStatus::Error);
    assert!(other.message.unwrap().contains("no such table"));
}

#[tokio::test]
async fn values_decode_by_storage_class() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    let outcome = run(
        &manager,
        "t",
        "SELECT id, name, price, raw FROM items ORDER BY id",
    )
    .await;
    let set = &outcome.result_sets[0];
    let names: Vec<_> = set.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "price", "raw"]);
    assert_eq!(
        set.rows[0],
        [
            CellValue::Int(1),
            CellValue::Text("pen".into()),
            CellValue::Float(1.5),
            CellValue::Bytes(vec![0x00, 0xff]),
        ]
    );
    assert_eq!(set.rows[1][2], CellValue::Null);
    assert!(!set.truncated);
}

#[tokio::test]
async fn empty_results_still_have_columns() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    let outcome = run(
        &manager,
        "t",
        "SELECT id, name AS label FROM items WHERE id > 99",
    )
    .await;
    assert_eq!(outcome.status, OutcomeStatus::Ok);
    let set = &outcome.result_sets[0];
    assert!(set.rows.is_empty());
    let names: Vec<_> = set.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "label"]);
}

#[tokio::test]
async fn big_results_are_kept_whole_and_read_in_batches() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    let total = FIRST_BATCH * 2 + 10;
    let sql = format!(
        "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < {total}) SELECT n FROM c"
    );
    let outcome = run(&manager, "t", &sql).await;
    let set = &outcome.result_sets[0];
    assert_eq!(set.rows.len(), FIRST_BATCH);
    assert_eq!(set.total_rows, total as u64);
    assert!(!set.truncated);

    // 后面的行往下滚时再取;执行了别的语句,之前的结果照样能取。
    assert_eq!(
        run(&manager, "t", "SELECT 1").await.status,
        OutcomeStatus::Ok
    );
    let rest = manager
        .result_rows("t", set.id, FIRST_BATCH, 5000, None)
        .await
        .unwrap();
    assert_eq!(rest.len(), total - FIRST_BATCH);
    assert_eq!(rest[0][0], CellValue::Int(FIRST_BATCH as i64 + 1));

    let sort = Some(ResultSort {
        column: 0,
        descending: true,
    });
    let top = manager.result_rows("t", set.id, 0, 2, sort).await.unwrap();
    assert_eq!(
        top,
        [
            vec![CellValue::Int(total as i64)],
            vec![CellValue::Int(total as i64 - 1)]
        ]
    );

    manager.release_results("t", &[set.id]);
    let error = manager
        .result_rows("t", set.id, 0, 10, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("重新执行"), "{error}");
}

#[tokio::test]
async fn result_columns_say_which_table_column_they_come_from() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    let source = |column: &str| {
        Some(ColumnSource {
            table: TableRef {
                database: "main".into(),
                schema: None,
                table: "items".into(),
            },
            column: column.into(),
        })
    };
    let outcome = run(
        &manager,
        "t",
        "SELECT id, name AS label, price * 2 AS doubled FROM items",
    )
    .await;
    let sources: Vec<_> = outcome.result_sets[0]
        .columns
        .iter()
        .map(|c| c.source.clone())
        .collect();
    assert_eq!(sources, [source("id"), source("name"), None]);

    // 没读到行的查询也带来源:空结果里一样能新增行。
    let empty = run(&manager, "t", "SELECT id, name FROM items WHERE id < 0").await;
    let set = &empty.result_sets[0];
    assert!(set.rows.is_empty());
    assert_eq!(set.columns[1].source, source("name"));
}

#[tokio::test]
async fn errors_keep_the_database_message() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    let outcome = run(&manager, "t", "INSERT INTO items (id) VALUES (3)").await;
    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(outcome.message.unwrap().contains("NOT NULL"));
    assert_eq!(
        run(&manager, "t", "SELECT 1").await.status,
        OutcomeStatus::Ok
    );
}

#[tokio::test]
async fn a_long_query_can_be_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager(&dir).await);
    let runner = manager.clone();
    let started = Instant::now();
    let long = tokio::spawn(async move {
        run(
            &runner,
            "t",
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c) SELECT count(*) FROM c",
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        manager.cancel_query("t").await.unwrap(),
        "正在执行，应该能取消"
    );
    let outcome = long.await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(
        !manager.cancel_query("t").await.unwrap(),
        "执行完了没东西可取消"
    );
    // 取消之后连接照常能用,标志不会留到下一条。
    assert_eq!(
        run(&manager, "t", "SELECT 1").await.status,
        OutcomeStatus::Ok
    );
}

#[tokio::test]
async fn closing_tabs_and_connections_drops_their_connections() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    run(&manager, "a", "SELECT 1").await;
    run(&manager, "b", "SELECT 1").await;
    assert!(!manager.cancel_query("unknown").await.unwrap());
    manager.close_editor("a").await;
    manager.close("c1").await;
    let error = manager
        .run_query("b", "c1", &scope(), "SELECT 1")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "not_connected");
}

#[tokio::test]
async fn cancelling_before_the_statement_starts_skips_it() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager(&dir).await);
    // 先建好这个标签页的连接,再在下一次执行刚开始准备时取消。
    run(&manager, "t", "SELECT 1").await;
    let runner = manager.clone();
    let pending = tokio::spawn(async move {
        run(&runner, "t", "INSERT INTO items (id, name) VALUES (9, 'x')").await
    });
    // 准备阶段很短,不一定赶得上;赶上了就必须没执行,没赶上就是照常执行完。
    let cancelled = manager.cancel_query("t").await.unwrap();
    let outcome = pending.await.unwrap();
    let inserted = run(&manager, "t", "SELECT count(*) FROM items WHERE id = 9").await;
    let count = inserted.result_sets[0].rows[0][0].clone();
    if outcome.status == OutcomeStatus::Cancelled {
        assert!(cancelled);
        assert_eq!(count, CellValue::Int(0), "准备阶段取消的语句不能执行");
    } else {
        assert_eq!(count, CellValue::Int(1));
    }
}

#[tokio::test]
async fn temporary_tables_are_flagged_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    run(
        &manager,
        "t",
        "CREATE TEMP TABLE scratch AS SELECT id, name FROM items",
    )
    .await;
    let outcome = run(&manager, "t", "SELECT id, name FROM scratch").await;
    let reason = outcome.result_sets[0].read_only.clone().unwrap_or_default();
    assert!(reason.contains("临时表 scratch"), "{reason}");
    assert_eq!(
        run(&manager, "t", "SELECT id FROM items").await.result_sets[0].read_only,
        None
    );
}

#[tokio::test]
async fn errors_say_where_in_the_statement_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).await;
    let outcome = run(&manager, "where", "SELECT * FORM items").await;
    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert_eq!(outcome.error_position, Some(9), "{:?}", outcome.message);
    let missing = run(&manager, "where", "SELECT * FROM missing").await;
    assert_eq!(missing.error_position, Some(14), "{:?}", missing.message);
}
