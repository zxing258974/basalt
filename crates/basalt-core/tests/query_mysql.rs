//! 查询标签页在 MySQL / MariaDB 上执行用户的 SQL。需要 BASALT_IT 选中目标,见 common。

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use basalt_core::query::{OutcomeStatus, QueryScope, StatementOutcome};
use basalt_core::session::SessionManager;
use basalt_core::value::CellValue;
use common::*;
use tokio::sync::OnceCell;

const DB: &str = "basalt_m3";
/// 要跑很久、被 KILL QUERY 时报错的查询。SLEEP() 不行:被中止时它只是返回 1,语句照常结束。
const LONG_QUERY: &str = "SELECT COUNT(*) FROM information_schema.COLUMNS a, information_schema.COLUMNS b, information_schema.COLUMNS c";

const FIXTURE: &str = r"
    CREATE DATABASE IF NOT EXISTS basalt_m3 CHARACTER SET utf8mb4;
    DROP TABLE IF EXISTS basalt_m3.items;
    CREATE TABLE basalt_m3.items (
      id INT PRIMARY KEY,
      name VARCHAR(20) NOT NULL,
      price DECIMAL(10, 2),
      added DATETIME,
      raw VARBINARY(4),
      big BIGINT UNSIGNED,
      flag BIT(3)
    );
    INSERT INTO basalt_m3.items VALUES
      (1, 'pen', 1.50, '2026-10-20 08:05:09', 0x00FF, 18446744073709551615, b'101'),
      (2, '本子', NULL, NULL, NULL, NULL, NULL);
    DROP PROCEDURE IF EXISTS basalt_m3.two_sets;
";

static FIXTURES: [OnceCell<()>; 3] = [
    OnceCell::const_new(),
    OnceCell::const_new(),
    OnceCell::const_new(),
];

async fn manager_for(index: usize, target: Target) -> SessionManager {
    FIXTURES[index]
        .get_or_init(|| async move {
            let pool = sqlx::MySqlPool::connect(&mysql_url(target))
                .await
                .expect("连接 MySQL 测试容器");
            sqlx::raw_sql(FIXTURE)
                .execute(&pool)
                .await
                .expect("建 M3 测试数据");
            pool.close().await;
        })
        .await;
    let manager = SessionManager::default();
    manager
        .open("c1", &params(target, TEST_PASSWORD, None))
        .await
        .unwrap();
    manager
}

fn targets() -> impl Iterator<Item = (usize, Target)> {
    MYSQL_FAMILY
        .into_iter()
        .enumerate()
        .filter(|(_, t)| selected(*t))
}

fn in_db() -> QueryScope {
    QueryScope {
        database: Some(DB.into()),
        schema: None,
    }
}

async fn run(manager: &SessionManager, editor: &str, sql: &str) -> StatementOutcome {
    manager
        .run_query(editor, "c1", &in_db(), sql)
        .await
        .unwrap()
}

#[tokio::test]
async fn session_state_survives_between_statements() {
    for (index, target) in targets() {
        let manager = manager_for(index, target).await;
        assert_eq!(
            run(&manager, "t", "SET @x = 41").await.status,
            OutcomeStatus::Ok
        );
        let answer = run(&manager, "t", "SELECT @x + 1 AS answer").await;
        assert_eq!(
            answer.result_sets[0].columns[0].name, "answer",
            "{}",
            target.name
        );
        assert_eq!(
            answer.result_sets[0].rows[0][0],
            CellValue::Int(42),
            "{}",
            target.name
        );
        // 别的标签页是另一条连接,看不到这个变量。
        let other = run(&manager, "u", "SELECT @x AS x").await;
        assert_eq!(
            other.result_sets[0].rows[0][0],
            CellValue::Null,
            "{}",
            target.name
        );
    }
}

#[tokio::test]
async fn text_protocol_values_keep_their_types() {
    for (index, target) in targets() {
        let manager = manager_for(index, target).await;
        let outcome = run(&manager, "t", "SELECT * FROM items ORDER BY id").await;
        let set = &outcome.result_sets[0];
        assert_eq!(
            set.rows[0],
            [
                CellValue::Int(1),
                CellValue::Text("pen".into()),
                CellValue::Decimal("1.50".into()),
                CellValue::DateTime("2026-10-20 08:05:09".into()),
                CellValue::Bytes(vec![0x00, 0xff]),
                CellValue::UInt(u64::MAX),
                CellValue::UInt(5),
            ],
            "{}",
            target.name
        );
        assert_eq!(set.rows[1][1], CellValue::Text("本子".into()));
        assert_eq!(set.columns[2].data_type, "decimal");
        assert_eq!(outcome.affected_rows, None);
    }
}

#[tokio::test]
async fn dml_reports_affected_rows_and_empty_selects_keep_columns() {
    for (index, target) in targets() {
        let manager = manager_for(index, target).await;
        run(&manager, "t", "CREATE TEMPORARY TABLE scratch (n INT)").await;
        let inserted = run(&manager, "t", "INSERT INTO scratch VALUES (1), (2)").await;
        assert_eq!(inserted.affected_rows, Some(2), "{}", target.name);
        let updated = run(&manager, "t", "UPDATE scratch SET n = n WHERE n > 5").await;
        assert_eq!(updated.affected_rows, Some(0));
        let empty = run(
            &manager,
            "t",
            "SELECT n, n * 2 AS twice FROM scratch WHERE n > 5",
        )
        .await;
        let names: Vec<_> = empty.result_sets[0]
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["n", "twice"], "{}", target.name);
        assert!(empty.result_sets[0].rows.is_empty());
    }
}

#[tokio::test]
async fn procedures_return_every_result_set() {
    for (index, target) in targets() {
        let manager = manager_for(index, target).await;
        let created = run(
            &manager,
            "t",
            "CREATE PROCEDURE two_sets() BEGIN SELECT 1 AS a; SELECT 2 AS b, 3 AS c; END",
        )
        .await;
        assert_eq!(created.status, OutcomeStatus::Ok, "{:?}", created.message);
        let called = run(&manager, "t", "CALL two_sets()").await;
        assert_eq!(called.result_sets.len(), 2, "{}", target.name);
        assert_eq!(called.result_sets[1].columns.len(), 2);
        run(&manager, "t", "DROP PROCEDURE two_sets").await;
    }
}

#[tokio::test]
async fn use_changes_the_reported_scope() {
    for (index, target) in targets() {
        let manager = manager_for(index, target).await;
        let switched = run(&manager, "t", "USE mysql").await;
        assert_eq!(
            switched.scope.database.as_deref(),
            Some("mysql"),
            "{}",
            target.name
        );
        // 前端跟着改了标签页的库,下一次带的就是 mysql。
        let scope = QueryScope {
            database: Some("mysql".into()),
            schema: None,
        };
        let current = manager
            .run_query("t", "c1", &scope, "SELECT DATABASE() AS db")
            .await
            .unwrap();
        assert_eq!(
            current.result_sets[0].rows[0][0],
            CellValue::Text("mysql".into())
        );
        // 标签页上换回 basalt_m3:执行前自动 USE。
        let back = run(&manager, "t", "SELECT DATABASE() AS db").await;
        assert_eq!(back.result_sets[0].rows[0][0], CellValue::Text(DB.into()));
    }
}

#[tokio::test]
async fn errors_carry_the_error_number() {
    for (index, target) in targets() {
        let manager = manager_for(index, target).await;
        let outcome = run(&manager, "t", "SELECT * FROM no_such_table").await;
        assert_eq!(outcome.status, OutcomeStatus::Error);
        let message = outcome.message.unwrap();
        assert!(message.starts_with("1146 - "), "{}: {message}", target.name);
    }
}

#[tokio::test]
async fn kill_query_cancels_a_long_statement() {
    for (index, target) in targets() {
        let manager = Arc::new(manager_for(index, target).await);
        let runner = manager.clone();
        let started = Instant::now();
        let long = tokio::spawn(async move { run(&runner, "t", LONG_QUERY).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        // 另一个标签页不受影响。
        assert_eq!(
            run(&manager, "u", "SELECT 1").await.status,
            OutcomeStatus::Ok
        );
        assert!(manager.cancel_query("t").await.unwrap(), "{}", target.name);
        let outcome = long.await.unwrap();
        assert_eq!(
            outcome.status,
            OutcomeStatus::Cancelled,
            "{}: {:?}",
            target.name,
            outcome.message
        );
        assert!(started.elapsed() < Duration::from_secs(20));
        assert_eq!(
            run(&manager, "t", "SELECT 1").await.status,
            OutcomeStatus::Ok
        );
    }
}

#[tokio::test]
async fn closing_the_connection_cancels_and_drops_editor_connections() {
    for (index, target) in targets() {
        let manager = Arc::new(manager_for(index, target).await);
        let runner = manager.clone();
        let long = tokio::spawn(async move { run(&runner, "t", LONG_QUERY).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let started = Instant::now();
        manager.close("c1").await;
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{}",
            target.name
        );
        assert_ne!(long.await.unwrap().status, OutcomeStatus::Ok);
    }
}
