//! 在查询标签页自己的连接上导出全部结果时,标签页的会话状态要原样留着。需要 BASALT_IT 选中 postgres。

mod common;
mod transfer_support;

use basalt_core::session::SessionManager;
use basalt_core::transfer::ExportFormat;
use common::*;
use transfer_support::*;

/// 在开着事务的查询标签页上重新执行查询导出,中途写不出来(GBK 表示不了 😀)停下时,
/// 标签页的事务不能跟着失效:之后的 COMMIT 要真的提交。
#[tokio::test]
async fn postgres_tab_transaction_survives_a_failed_export() {
    use basalt_core::query::{OutcomeStatus, QueryScope};
    use basalt_core::transfer::export::{ExportRequest, ExportSource, run_export};
    use basalt_core::transfer::progress::{CancelFlag, ignore};
    use basalt_core::transfer::{BinaryEncoding, CsvOptions, ExportOptions, TextEncoding};

    if !selected(POSTGRES) {
        return;
    }
    ensure_postgres_database(&postgres_url("postgres"), FIXTURE_DB).await;
    let pool = sqlx::PgPool::connect(&postgres_url(FIXTURE_DB))
        .await
        .unwrap();
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS public.m6_tx; CREATE TABLE public.m6_tx (id int PRIMARY KEY)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let sessions = SessionManager::default();
    sessions
        .open(
            CONNECTION,
            &params(POSTGRES, TEST_PASSWORD, Some(FIXTURE_DB)),
        )
        .await
        .unwrap();
    let scope = QueryScope {
        database: Some(FIXTURE_DB.into()),
        schema: Some("public".into()),
    };
    for sql in ["BEGIN", "INSERT INTO m6_tx VALUES (1)"] {
        let outcome = sessions
            .run_query("tab", CONNECTION, &scope, sql)
            .await
            .unwrap();
        assert_eq!(outcome.status, OutcomeStatus::Ok, "{sql}");
    }
    // 第一行超过 8 KB,服务器会先把它发过来;第二行要睡 5 秒——导出停下时语句还在跑,一定会被中止。
    let dir = tempfile::tempdir().unwrap();
    let request = ExportRequest {
        connection_id: CONNECTION.into(),
        source: ExportSource::Query {
            editor_id: "tab".into(),
            scope: scope.clone(),
            sql: "SELECT repeat('笑😀', 4000) AS v UNION ALL SELECT pg_sleep(5)::text".into(),
        },
        options: ExportOptions {
            format: ExportFormat::Csv(CsvOptions {
                encoding: TextEncoding::Gbk,
                ..CsvOptions::default()
            }),
            binary: BinaryEncoding::Hex,
        },
        path: dir.path().join("x.csv").to_string_lossy().into_owned(),
    };
    let started = std::time::Instant::now();
    let error = run_export(&sessions, "job", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_data", "{error}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "语句应该被中止，不该等它睡完"
    );
    let outcome = sessions
        .run_query("tab", CONNECTION, &scope, "COMMIT")
        .await
        .unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Ok);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.m6_tx")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "标签页的事务应该照常提交");
    pool.close().await;
    sessions.close(CONNECTION).await;
}
