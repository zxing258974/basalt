//! MySQL 5.7 上表定义缓存不够用时读视图。需要 BASALT_IT 选中 mysql57(见 common)。
//!
//! 打开着的表(table_open_cache 里的)用到的表定义比 table_definition_cache 多时,视图的定义一用完
//! 就被扔掉,下次重读成新版本:预备语句执行时发现版本变了,重新预备三次都一样,次次报 1615
//! (Prepared statement needs to be re-prepared)。表多的云数据库上常见。表数据页和导出要照样读得出来。
//!
//! 要改服务器的全局设置,单独成一个测试程序(cargo 一个接一个跑测试程序),跑完恢复。

mod common;
mod transfer_support;

use basalt_core::session::SessionManager;
use basalt_core::table::{PageRequest, TableRef};
use basalt_core::transfer::{CsvOptions, ExportFormat};
use basalt_core::value::CellValue;
use common::*;
use sqlx::MySqlPool;
use sqlx::mysql::{MySqlDatabaseError, MySqlPoolOptions};
use transfer_support::*;

const DB: &str = "basalt_reprepare";
/// 同时把表打开的连接数,和 table_open_cache_instances 的默认值一样:每条连接落在不同的分区里。
const OPENERS: usize = 16;
/// 测试期间把 table_open_cache 调到这么大,让打开的表都留在缓存里。
const OPEN_CACHE: u64 = 4000;

fn view() -> TableRef {
    TableRef {
        database: DB.into(),
        schema: None,
        table: "v".into(),
    }
}

/// 建库、`count` 张小表和一个视图。
async fn create_fixture(pool: &MySqlPool, count: usize) {
    let mut sql = format!(
        "DROP DATABASE IF EXISTS {DB}; CREATE DATABASE {DB} CHARACTER SET utf8mb4; USE {DB};"
    );
    for i in 1..=count {
        sql.push_str(&format!(
            "CREATE TABLE t{i} (id INT PRIMARY KEY, name VARCHAR(20));"
        ));
    }
    sql.push_str(
        "INSERT INTO t1 VALUES (1, 'a'), (2, 'b');
         INSERT INTO t2 VALUES (1, 'x'), (2, 'y');
         CREATE VIEW v AS SELECT t1.id, t1.name, t2.name AS other FROM t1 JOIN t2 ON t2.id = t1.id;",
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(pool)
        .await
        .expect("建视图测试数据");
}

/// 多条连接各从不同的地方起把所有表打开一遍:每个缓存分区留下的是不同的一段,合起来几乎是全部。
async fn open_every_table(pool: &MySqlPool, count: usize) {
    let openers = (0..OPENERS).map(|k| {
        let pool = pool.clone();
        async move {
            let mut conn = pool.acquire().await.expect("连接");
            let start = k * count / OPENERS;
            let sql: String = (0..count)
                .map(|i| format!("SELECT 1 FROM {DB}.t{} LIMIT 0;", (start + i) % count + 1))
                .collect();
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(&mut *conn)
                .await
                .expect("打开所有表");
            conn
        }
    });
    // 连接都拿着,等全部打开完再还:同时拿 16 条,才会落在 16 个分区里。
    drop(futures_util::future::join_all(openers).await);
}

async fn check_view_reads(pool: MySqlPool) {
    let prepared = sqlx::query("SELECT * FROM basalt_reprepare.v")
        .fetch_all(&pool)
        .await;
    let number = prepared
        .as_ref()
        .err()
        .and_then(|e| e.as_database_error())
        .and_then(|e| e.try_downcast_ref::<MySqlDatabaseError>())
        .map(MySqlDatabaseError::number);
    assert_eq!(
        number,
        Some(1615),
        "复现条件没满足:预备语句读视图本该报 1615,实际 {prepared:?}"
    );

    let sessions = SessionManager::default();
    sessions
        .open(CONNECTION, &params(MYSQL57, TEST_PASSWORD, None))
        .await
        .unwrap();
    let session = sessions.get(CONNECTION).await.unwrap();
    let request = PageRequest {
        offset: 0,
        limit: 100,
        sort: None,
        filter: Some("id > 0".into()),
    };
    let page = session
        .read_table(&view(), &request)
        .await
        .expect("表数据页读视图");
    let mut rows = page.rows;
    rows.sort_by_key(|row| format!("{:?}", row[0]));
    assert_eq!(
        rows,
        vec![
            vec![
                CellValue::Int(1),
                CellValue::Text("a".into()),
                CellValue::Text("x".into())
            ],
            vec![
                CellValue::Int(2),
                CellValue::Text("b".into()),
                CellValue::Text("y".into())
            ],
        ]
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.csv");
    let exported = export_table(
        &sessions,
        &view(),
        ExportFormat::Csv(CsvOptions::default()),
        &path,
    )
    .await;
    assert_eq!(exported, 2, "导出视图");
    sessions.close(CONNECTION).await;
}

#[tokio::test]
async fn views_still_read_when_table_definitions_overflow_the_cache() {
    if !selected(MYSQL57) {
        return;
    }
    let pool = MySqlPoolOptions::new()
        .max_connections(OPENERS as u32 + 2)
        .connect(&mysql_url(MYSQL57))
        .await
        .expect("连接 MySQL 5.7 测试容器");
    let (open_cache, definitions): (u64, u64) =
        sqlx::query_as("SELECT @@table_open_cache, @@table_definition_cache")
            .fetch_one(&pool)
            .await
            .unwrap();
    let count = usize::try_from(definitions).unwrap() + 300;
    create_fixture(&pool, count).await;

    // 从调大 table_open_cache 起,中途出错也要把服务器恢复原样,再报出来。
    let reproduce = {
        let pool = pool.clone();
        async move {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "SET GLOBAL table_open_cache = {OPEN_CACHE}"
            )))
            .execute(&pool)
            .await
            .expect("调大 table_open_cache");
            open_every_table(&pool, count).await;
            check_view_reads(pool).await;
        }
    };
    let checked = tokio::spawn(reproduce).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "FLUSH TABLES; SET GLOBAL table_open_cache = {open_cache}; DROP DATABASE {DB};"
    )))
    .execute(&pool)
    .await
    .expect("恢复 table_open_cache、删掉测试库");
    pool.close().await;
    if let Err(error) = checked {
        std::panic::resume_unwind(error.into_panic());
    }
}
