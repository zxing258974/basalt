//! 查询标签页在 PostgreSQL 上执行用户的 SQL。需要 BASALT_IT 选中 postgres,见 common。

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use basalt_core::query::{OutcomeStatus, QueryScope, StatementOutcome};
use basalt_core::session::SessionManager;
use basalt_core::value::CellValue;
use common::*;
use tokio::sync::OnceCell;

const DB: &str = "basalt_m3";

const FIXTURE: &str = r#"
    DROP SCHEMA IF EXISTS app CASCADE;
    CREATE SCHEMA app;
    CREATE TYPE app.mood AS ENUM ('sad', 'happy');
    CREATE TABLE app.items (
      id bigint PRIMARY KEY,
      name text NOT NULL,
      price numeric(40, 20),
      flag boolean,
      tags text[],
      doc jsonb,
      raw bytea,
      feeling app.mood,
      seen timestamp
    );
    INSERT INTO app.items VALUES
      (9007199254740993, 'pen', '12345678901234567890.01234567890123456789', true, '{a,"b c"}',
       '{"k": 1}', '\x00ff', 'happy', '2026-10-20 08:05:09'),
      (2, '本子', NULL, NULL, NULL, NULL, NULL, NULL, NULL);
"#;

static FIXTURE_READY: OnceCell<()> = OnceCell::const_new();

async fn manager() -> Option<SessionManager> {
    if !selected(POSTGRES) {
        return None;
    }
    FIXTURE_READY
        .get_or_init(|| async {
            let admin = sqlx::PgPool::connect(&postgres_url("postgres"))
                .await
                .expect("连接 PG 测试容器");
            let exists: Option<i32> =
                sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
                    .bind(DB)
                    .fetch_optional(&admin)
                    .await
                    .unwrap();
            if exists.is_none() {
                sqlx::raw_sql("CREATE DATABASE basalt_m3")
                    .execute(&admin)
                    .await
                    .unwrap();
            }
            admin.close().await;
            let db = sqlx::PgPool::connect(&postgres_url(DB)).await.unwrap();
            sqlx::raw_sql(FIXTURE)
                .execute(&db)
                .await
                .expect("建 M3 测试数据");
            db.close().await;
        })
        .await;
    let manager = SessionManager::default();
    manager
        .open("c1", &params(POSTGRES, TEST_PASSWORD, None))
        .await
        .unwrap();
    Some(manager)
}

fn scope(database: &str, schema: Option<&str>) -> QueryScope {
    QueryScope {
        database: Some(database.into()),
        schema: schema.map(str::to_owned),
    }
}

async fn run(manager: &SessionManager, editor: &str, sql: &str) -> StatementOutcome {
    manager
        .run_query(editor, "c1", &scope(DB, Some("app")), sql)
        .await
        .unwrap()
}

#[tokio::test]
async fn schema_goes_on_the_search_path() {
    let Some(manager) = manager().await else {
        return;
    };
    let outcome = run(
        &manager,
        "t",
        "SELECT current_schema() AS s, count(*) AS n FROM items",
    )
    .await;
    assert_eq!(outcome.status, OutcomeStatus::Ok, "{:?}", outcome.message);
    assert_eq!(
        outcome.result_sets[0].rows[0][0],
        CellValue::Text("app".into())
    );
    assert_eq!(outcome.result_sets[0].rows[0][1], CellValue::Int(2));
    assert_eq!(outcome.scope, scope(DB, Some("app")));
}

#[tokio::test]
async fn text_results_decode_into_typed_cells() {
    let Some(manager) = manager().await else {
        return;
    };
    let outcome = run(&manager, "t", "SELECT * FROM items ORDER BY id DESC").await;
    let row = &outcome.result_sets[0].rows[0];
    assert_eq!(row[0], CellValue::Int(9_007_199_254_740_993));
    assert_eq!(row[1], CellValue::Text("pen".into()));
    assert_eq!(
        row[2],
        CellValue::Decimal("12345678901234567890.01234567890123456789".into())
    );
    assert_eq!(row[3], CellValue::Bool(true));
    assert_eq!(
        row[4],
        CellValue::Text("{a,\"b c\"}".into()),
        "数组按 PostgreSQL 的写法显示"
    );
    assert_eq!(row[5], CellValue::Json("{\"k\": 1}".into()));
    assert_eq!(row[6], CellValue::Bytes(vec![0x00, 0xff]));
    assert_eq!(row[7], CellValue::Text("happy".into()), "枚举也能显示");
    assert_eq!(row[8], CellValue::DateTime("2026-10-20 08:05:09".into()));
    let types: Vec<_> = outcome.result_sets[0]
        .columns
        .iter()
        .map(|c| c.data_type.as_str())
        .collect();
    assert_eq!(&types[..4], ["int8", "text", "numeric", "bool"]);
}

#[tokio::test]
async fn transactions_and_temp_tables_stay_on_the_tab() {
    let Some(manager) = manager().await else {
        return;
    };
    run(&manager, "t", "CREATE TEMP TABLE scratch (n int)").await;
    assert_eq!(run(&manager, "t", "BEGIN").await.status, OutcomeStatus::Ok);
    let inserted = run(
        &manager,
        "t",
        "INSERT INTO scratch SELECT generate_series(1, 3)",
    )
    .await;
    assert_eq!(inserted.affected_rows, Some(3));
    let empty = run(&manager, "t", "SELECT n FROM scratch WHERE n > 10").await;
    assert_eq!(empty.result_sets[0].columns[0].name, "n");
    assert!(empty.result_sets[0].rows.is_empty());
    assert_eq!(
        run(&manager, "t", "ROLLBACK").await.status,
        OutcomeStatus::Ok
    );
    let counted = run(&manager, "t", "SELECT count(*) FROM scratch").await;
    assert_eq!(counted.result_sets[0].rows[0][0], CellValue::Int(0));
    let other = run(&manager, "u", "SELECT * FROM scratch").await;
    assert_eq!(other.status, OutcomeStatus::Error);
}

#[tokio::test]
async fn dollar_quoted_bodies_run_as_one_statement() {
    let Some(manager) = manager().await else {
        return;
    };
    let outcome = run(
        &manager,
        "t",
        "DO $$ BEGIN PERFORM 1; RAISE NOTICE 'x'; END $$",
    )
    .await;
    assert_eq!(outcome.status, OutcomeStatus::Ok, "{:?}", outcome.message);
    assert!(outcome.result_sets.is_empty());
    assert_eq!(outcome.affected_rows, None);
}

#[tokio::test]
async fn errors_include_detail_and_hint() {
    let Some(manager) = manager().await else {
        return;
    };
    let outcome = run(
        &manager,
        "t",
        "INSERT INTO items (id, name) VALUES (2, 'dup')",
    )
    .await;
    assert_eq!(outcome.status, OutcomeStatus::Error);
    let message = outcome.message.unwrap();
    assert!(message.contains("duplicate key"), "{message}");
    assert!(message.contains("详情："), "{message}");
}

#[tokio::test]
async fn switching_database_reconnects() {
    let Some(manager) = manager().await else {
        return;
    };
    let on_postgres = manager
        .run_query(
            "t",
            "c1",
            &scope("postgres", None),
            "SELECT current_database()",
        )
        .await
        .unwrap();
    assert_eq!(
        on_postgres.result_sets[0].rows[0][0],
        CellValue::Text("postgres".into())
    );
    assert_eq!(on_postgres.scope.database.as_deref(), Some("postgres"));
    assert_eq!(
        on_postgres.scope.schema.as_deref(),
        Some("public"),
        "没选 schema 时报当前的"
    );
    let on_m3 = run(&manager, "t", "SELECT current_database()").await;
    assert_eq!(on_m3.result_sets[0].rows[0][0], CellValue::Text(DB.into()));
}

#[tokio::test]
async fn set_search_path_is_reported_back() {
    let Some(manager) = manager().await else {
        return;
    };
    let outcome = run(&manager, "t", "SET search_path TO public").await;
    assert_eq!(outcome.scope.schema.as_deref(), Some("public"));
}

#[tokio::test]
async fn pg_cancel_backend_cancels_a_long_statement() {
    let Some(manager) = manager().await else {
        return;
    };
    let manager = Arc::new(manager);
    let runner = manager.clone();
    let started = Instant::now();
    let long = tokio::spawn(async move { run(&runner, "t", "SELECT pg_sleep(30)").await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(manager.cancel_query("t").await.unwrap());
    let outcome = long.await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Cancelled);
    // 取消请求可能赶在语句真正开始之前(准备连接时),那就根本没执行。
    let message = outcome.message.unwrap();
    assert!(
        message.contains("cancel") || message.contains("取消"),
        "{message}"
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(
        run(&manager, "t", "SELECT 1").await.status,
        OutcomeStatus::Ok
    );
}

#[tokio::test]
async fn empty_results_see_new_columns_after_ddl() {
    let Some(manager) = manager().await else {
        return;
    };
    run(&manager, "t", "CREATE TEMP TABLE grows (a int)").await;
    let before = run(&manager, "t", "SELECT * FROM grows").await;
    assert_eq!(before.result_sets[0].columns.len(), 1);
    run(&manager, "t", "ALTER TABLE grows ADD COLUMN b text").await;
    let after = run(&manager, "t", "SELECT * FROM grows").await;
    let names: Vec<_> = after.result_sets[0]
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["a", "b"], "预备语句缓存不能带着旧的列");
}

#[tokio::test]
async fn switching_database_says_the_session_is_new() {
    let Some(manager) = manager().await else {
        return;
    };
    run(&manager, "t", "SELECT 1").await;
    let moved = manager
        .run_query("t", "c1", &scope("postgres", None), "SELECT 1")
        .await
        .unwrap();
    assert!(moved.notice.unwrap().contains("新开一条连接"));
    let again = manager
        .run_query("t", "c1", &scope("postgres", None), "SELECT 1")
        .await
        .unwrap();
    assert_eq!(again.notice, None);
}

#[tokio::test]
async fn copy_from_stdin_is_explained_and_the_tab_recovers() {
    let Some(manager) = manager().await else {
        return;
    };
    run(&manager, "t", "CREATE TEMP TABLE c (n int)").await;
    let copy = run(&manager, "t", "COPY c FROM STDIN").await;
    assert_eq!(copy.status, OutcomeStatus::Error);
    assert!(copy.message.unwrap().contains("SQL 编辑器不支持"));
    assert_eq!(
        run(&manager, "t", "SELECT 1").await.status,
        OutcomeStatus::Ok
    );
}
