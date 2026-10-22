//! 表的维护在真库上:MySQL 系四种语句都带回数据库的消息(InnoDB 不支持 REPAIR 也是一条消息,MyISAM 能修),
//! 归类和真实返回对得上:做完了的不算「发现问题」(含 MariaDB 收集统计时多给的那一行),表不存在才算;
//! 别的会话占着表时维护照数据库原来的等法等下去,不会几秒就放弃;
//! PostgreSQL 的 VACUUM、REINDEX 量出前后大小,ANALYZE 之后有了行数估算;SQLite 删掉大量数据后 VACUUM
//! 让库文件变小。各家没有的维护直接拒绝。
//! MySQL / MariaDB / PostgreSQL 需要 BASALT_IT 选中目标(见 common);SQLite 总是跑。

mod common;

use basalt_core::session::SessionManager;
use basalt_core::table::TableRef;
use basalt_core::table_ops::maintenance::{Maintenance, MaintenanceOutcome, MessageKind};
use common::*;
use sqlx::Connection;

const CONNECTION: &str = "c1";

const MYSQL_FIXTURE: &str = "
    CREATE DATABASE IF NOT EXISTS basalt_ops_more CHARACTER SET utf8mb4;
    DROP TABLE IF EXISTS basalt_ops_more.more_tuned, basalt_ops_more.more_myisam;
    CREATE TABLE basalt_ops_more.more_tuned (id INT PRIMARY KEY, name VARCHAR(20)) ENGINE=InnoDB;
    INSERT INTO basalt_ops_more.more_tuned VALUES (1, 'a'), (2, 'b');
    CREATE TABLE basalt_ops_more.more_myisam (id INT PRIMARY KEY) ENGINE=MyISAM;
";

const PG_FIXTURE: &str = "
    DROP TABLE IF EXISTS public.more_tuned;
    CREATE TABLE public.more_tuned (id integer PRIMARY KEY, payload text);
    INSERT INTO public.more_tuned SELECT n, repeat('x', 200) FROM generate_series(1, 5000) n;
    CREATE INDEX more_tuned_payload ON public.more_tuned (payload);
    DELETE FROM public.more_tuned;
";

fn has(outcome: &MaintenanceOutcome, level: &str, text: &str) -> bool {
    outcome
        .messages
        .iter()
        .any(|m| m.level == level && m.text.contains(text))
}

#[tokio::test]
async fn mysql_family_reports_the_database_messages() {
    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let pool = sqlx::MySqlPool::connect(&mysql_url(target)).await.unwrap();
        sqlx::raw_sql(MYSQL_FIXTURE).execute(&pool).await.unwrap();
        pool.close().await;
        let sessions = SessionManager::default();
        sessions
            .open(CONNECTION, &params(target, TEST_PASSWORD, None))
            .await
            .unwrap();
        let session = sessions.get(CONNECTION).await.unwrap();
        let table = |name: &str| TableRef {
            database: "basalt_ops_more".into(),
            schema: None,
            table: name.into(),
        };
        let label = target.name;
        let tuned = table("more_tuned");
        for operation in [
            Maintenance::Analyze,
            Maintenance::Check,
            Maintenance::Optimize,
        ] {
            let outcome = session.maintain_table(&tuned, operation).await.unwrap();
            assert!(
                has(&outcome, "status", "OK"),
                "{label} {operation:?}：{outcome:?}"
            );
            assert!(!outcome.has_problem(), "{label} {operation:?}：{outcome:?}");
            assert_eq!(outcome.size, None, "{label}");
        }
        let optimized = session
            .maintain_table(&tuned, Maintenance::Optimize)
            .await
            .unwrap();
        assert!(
            has(&optimized, "note", "recreate"),
            "{label}：InnoDB 的 OPTIMIZE 是重建表 {optimized:?}"
        );
        assert_eq!(
            optimized.sql,
            "OPTIMIZE TABLE `basalt_ops_more`.`more_tuned`"
        );
        let repaired = session
            .maintain_table(&tuned, Maintenance::Repair)
            .await
            .unwrap();
        assert!(
            has(&repaired, "note", "support repair"),
            "{label}：InnoDB 不支持 REPAIR {repaired:?}"
        );
        // 不支持只是一条提示,不是「发现问题」。
        assert!(!repaired.has_problem(), "{label}：{repaired:?}");
        let myisam = session
            .maintain_table(&table("more_myisam"), Maintenance::Repair)
            .await
            .unwrap();
        assert!(has(&myisam, "status", "OK"), "{label}：{myisam:?}");
        assert!(!myisam.has_problem(), "{label}");
        // 表不存在也是以消息报回来的,不是语句出错。
        let missing = session
            .maintain_table(&table("more_missing"), Maintenance::Check)
            .await
            .unwrap();
        assert!(
            has(&missing, "error", "more_missing"),
            "{label}：{missing:?}"
        );
        assert!(missing.has_problem(), "{label}：表不存在是问题");
        let refused = session
            .maintain_table(&tuned, Maintenance::Vacuum)
            .await
            .unwrap_err();
        assert_eq!(refused.code(), "unsupported", "{label}");
    }
}

#[tokio::test]
async fn postgres_vacuums_analyzes_and_reindexes() {
    if !selected(POSTGRES) {
        return;
    }
    let pool = postgres_fixture_pool().await;
    sqlx::raw_sql(PG_FIXTURE).execute(&pool).await.unwrap();
    let sessions = SessionManager::default();
    sessions
        .open(
            CONNECTION,
            &params(POSTGRES, TEST_PASSWORD, Some(FIXTURE_DB)),
        )
        .await
        .unwrap();
    let session = sessions.get(CONNECTION).await.unwrap();
    let table = TableRef {
        database: FIXTURE_DB.into(),
        schema: Some("public".into()),
        table: "more_tuned".into(),
    };
    // 全删掉之后 VACUUM 通常能把末尾的空页还给系统;同一个实例上别处开着的长事务会拦住它,只要求不变大。
    let vacuumed = session
        .maintain_table(&table, Maintenance::Vacuum)
        .await
        .unwrap();
    let size = vacuumed.size.expect("VACUUM 量了大小");
    assert!(size.before > 0 && size.after <= size.before, "{size:?}");
    assert!(vacuumed.messages.is_empty());
    assert_eq!(vacuumed.sql, "VACUUM \"public\".\"more_tuned\"");

    sqlx::raw_sql("INSERT INTO public.more_tuned SELECT n, 'y' FROM generate_series(1, 300) n")
        .execute(&pool)
        .await
        .unwrap();
    let analyzed = session
        .maintain_table(&table, Maintenance::Analyze)
        .await
        .unwrap();
    assert_eq!(analyzed.size, None);
    let estimate: f32 =
        sqlx::query_scalar("SELECT reltuples FROM pg_class WHERE relname = 'more_tuned'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(estimate, 300.0, "ANALYZE 更新了行数估算");

    let reindexed = session
        .maintain_table(&table, Maintenance::Reindex)
        .await
        .unwrap();
    assert!(reindexed.size.is_some());
    let refused = session
        .maintain_table(&table, Maintenance::Optimize)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), "unsupported");

    // 不是所有者:数据库只会警告着跳过,这里先拦下来报错。
    sqlx::raw_sql(
        "DO $$ BEGIN
           IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'basalt_more_reader') THEN
             CREATE ROLE basalt_more_reader LOGIN PASSWORD 'reader';
           END IF;
         END $$;
         GRANT SELECT ON public.more_tuned TO basalt_more_reader;",
    )
    .execute(&pool)
    .await
    .unwrap();
    let reader = SessionManager::default();
    let mut connect = params(POSTGRES, "reader", Some(FIXTURE_DB));
    connect.username = "basalt_more_reader".into();
    reader.open(CONNECTION, &connect).await.unwrap();
    let reader = reader.get(CONNECTION).await.unwrap();
    for operation in [Maintenance::Vacuum, Maintenance::Analyze] {
        let error = reader.maintain_table(&table, operation).await.unwrap_err();
        assert_eq!(error.code(), "database", "{operation:?}");
        assert!(error.to_string().contains("没有权限"), "{error}");
    }
    pool.close().await;
}

/// 库的所有者能清理库里别人的表(共享的系统表除外);分区表的大小是各个分区加起来,不是 0。
#[tokio::test]
async fn postgres_database_owners_maintain_and_partitions_are_measured() {
    if !selected(POSTGRES) {
        return;
    }
    let admin = sqlx::PgPool::connect(&postgres_url("postgres"))
        .await
        .unwrap();
    sqlx::raw_sql(
        "DO $$ BEGIN
           IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'basalt_more_owner') THEN
             CREATE ROLE basalt_more_owner LOGIN PASSWORD 'owner';
           END IF;
         END $$;",
    )
    .execute(&admin)
    .await
    .unwrap();
    // 别的测试可能已经建过这个库。
    let _ = sqlx::raw_sql("CREATE DATABASE basalt_more_owned OWNER basalt_more_owner")
        .execute(&admin)
        .await;
    admin.close().await;
    let owned = sqlx::PgPool::connect(&postgres_url("basalt_more_owned"))
        .await
        .unwrap();
    // 表的所有者是 postgres,不是库的所有者。
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS public.more_parted, public.more_plain;
         CREATE TABLE public.more_plain (id integer);
         CREATE TABLE public.more_parted (id integer, payload text) PARTITION BY RANGE (id);
         CREATE TABLE public.more_parted_a PARTITION OF public.more_parted FOR VALUES FROM (0) TO (1000);
         CREATE TABLE public.more_parted_b PARTITION OF public.more_parted FOR VALUES FROM (1000) TO (2000);
         INSERT INTO public.more_parted SELECT n, repeat('x', 100) FROM generate_series(0, 1999) n;",
    )
    .execute(&owned)
    .await
    .unwrap();
    owned.close().await;

    let sessions = SessionManager::default();
    let mut connect = params(POSTGRES, "owner", Some("basalt_more_owned"));
    connect.username = "basalt_more_owner".into();
    sessions.open(CONNECTION, &connect).await.unwrap();
    let session = sessions.get(CONNECTION).await.unwrap();
    let table = |name: &str| TableRef {
        database: "basalt_more_owned".into(),
        schema: Some("public".into()),
        table: name.into(),
    };
    for operation in [Maintenance::Vacuum, Maintenance::Analyze] {
        session
            .maintain_table(&table("more_plain"), operation)
            .await
            .unwrap();
    }

    let postgres = SessionManager::default();
    postgres
        .open(
            CONNECTION,
            &params(POSTGRES, TEST_PASSWORD, Some("basalt_more_owned")),
        )
        .await
        .unwrap();
    let vacuumed = postgres
        .get(CONNECTION)
        .await
        .unwrap()
        .maintain_table(&table("more_parted"), Maintenance::Vacuum)
        .await
        .unwrap();
    let size = vacuumed.size.expect("VACUUM 量了大小");
    assert!(size.before > 0 && size.after > 0, "{size:?}");
}

/// MariaDB 收集引擎无关统计时(use_stat_tables 为 PREFERABLY,很多服务器这么配),ANALYZE 先给一行
/// 「Engine-independent statistics collected」再给 OK:这一行不是问题。测试里临时改全局设置,用完改回去。
#[tokio::test]
async fn mariadb_statistics_line_is_not_a_problem() {
    if !selected(MARIADB) {
        return;
    }
    let admin = sqlx::MySqlPool::connect(&mysql_url(MARIADB)).await.unwrap();
    sqlx::raw_sql(
        "CREATE DATABASE IF NOT EXISTS basalt_ops_more CHARACTER SET utf8mb4;
         USE basalt_ops_more;
         DROP TABLE IF EXISTS basalt_ops_more.more_stats;
         CREATE TABLE basalt_ops_more.more_stats (id INT PRIMARY KEY, v VARCHAR(20), KEY (v)) ENGINE=InnoDB;
         INSERT INTO basalt_ops_more.more_stats SELECT seq, CONCAT('x', seq % 50) FROM seq_1_to_2000;",
    )
    .execute(&admin)
    .await
    .unwrap();
    let previous: String = sqlx::query_scalar("SELECT CAST(@@GLOBAL.use_stat_tables AS CHAR)")
        .fetch_one(&admin)
        .await
        .unwrap();
    sqlx::raw_sql("SET GLOBAL use_stat_tables = 'PREFERABLY'")
        .execute(&admin)
        .await
        .unwrap();
    let sessions = SessionManager::default();
    sessions
        .open(CONNECTION, &params(MARIADB, TEST_PASSWORD, None))
        .await
        .unwrap();
    let outcome = sessions
        .get(CONNECTION)
        .await
        .unwrap()
        .maintain_table(
            &TableRef {
                database: "basalt_ops_more".into(),
                schema: None,
                table: "more_stats".into(),
            },
            Maintenance::Analyze,
        )
        .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "SET GLOBAL use_stat_tables = '{previous}'"
    )))
    .execute(&admin)
    .await
    .unwrap();
    admin.close().await;
    let outcome = outcome.unwrap();
    let kinds: Vec<(&str, MessageKind)> = outcome
        .messages
        .iter()
        .map(|m| (m.text.as_str(), m.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("Engine-independent statistics collected", MessageKind::Info),
            ("OK", MessageKind::Success)
        ]
    );
    assert!(!outcome.has_problem());
}

/// 别的会话占着表、过了 5 秒(改名、删表等锁的上限)才放手:维护照常等到它放手再做完,不半途放弃。
#[tokio::test]
async fn maintenance_waits_for_locks_like_the_database_does() {
    const HOLD: std::time::Duration = std::time::Duration::from_secs(7);
    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let url = format!("{}basalt_ops_more", mysql_url(target));
        let admin = sqlx::MySqlPool::connect(&mysql_url(target)).await.unwrap();
        sqlx::raw_sql(
            "CREATE DATABASE IF NOT EXISTS basalt_ops_more CHARACTER SET utf8mb4;
             DROP TABLE IF EXISTS basalt_ops_more.more_locked;
             CREATE TABLE basalt_ops_more.more_locked (id INT PRIMARY KEY) ENGINE=InnoDB;
             INSERT INTO basalt_ops_more.more_locked VALUES (1), (2);",
        )
        .execute(&admin)
        .await
        .unwrap();
        admin.close().await;
        let sessions = SessionManager::default();
        sessions
            .open(CONNECTION, &params(target, TEST_PASSWORD, None))
            .await
            .unwrap();
        let session = sessions.get(CONNECTION).await.unwrap();
        let mut holder = sqlx::MySqlConnection::connect(&url).await.unwrap();
        sqlx::raw_sql("BEGIN; SELECT * FROM more_locked;")
            .execute(&mut holder)
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let task = tokio::spawn(async move {
            let locked = TableRef {
                database: "basalt_ops_more".into(),
                schema: None,
                table: "more_locked".into(),
            };
            session.maintain_table(&locked, Maintenance::Optimize).await
        });
        tokio::time::sleep(HOLD).await;
        sqlx::raw_sql("ROLLBACK")
            .execute(&mut holder)
            .await
            .unwrap();
        let outcome = task.await.unwrap().unwrap();
        assert!(!outcome.has_problem(), "{}：{outcome:?}", target.name);
        assert!(started.elapsed() >= HOLD, "{}：等到了对方放手", target.name);
    }
    if !selected(POSTGRES) {
        return;
    }
    let pool = postgres_fixture_pool().await;
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS public.more_locked;
         CREATE TABLE public.more_locked (id integer PRIMARY KEY);
         INSERT INTO public.more_locked VALUES (1), (2);",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let sessions = SessionManager::default();
    sessions
        .open(
            CONNECTION,
            &params(POSTGRES, TEST_PASSWORD, Some(FIXTURE_DB)),
        )
        .await
        .unwrap();
    let session = sessions.get(CONNECTION).await.unwrap();
    let mut holder = sqlx::PgConnection::connect(&postgres_url(FIXTURE_DB))
        .await
        .unwrap();
    sqlx::raw_sql("BEGIN; LOCK TABLE public.more_locked IN EXCLUSIVE MODE;")
        .execute(&mut holder)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let task = tokio::spawn(async move {
        let locked = TableRef {
            database: FIXTURE_DB.into(),
            schema: Some("public".into()),
            table: "more_locked".into(),
        };
        session.maintain_table(&locked, Maintenance::Vacuum).await
    });
    tokio::time::sleep(HOLD).await;
    sqlx::raw_sql("ROLLBACK")
        .execute(&mut holder)
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    assert!(started.elapsed() >= HOLD, "PostgreSQL：等到了对方放手");
}

#[tokio::test]
async fn sqlite_vacuum_shrinks_the_file_and_analyze_writes_statistics() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(
        dir.path(),
        "tuned.db",
        "CREATE TABLE tuned (id INTEGER PRIMARY KEY, payload BLOB);
         CREATE INDEX tuned_payload ON tuned (payload);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
         INSERT INTO tuned SELECT i, randomblob(500) FROM n;
         DELETE FROM tuned WHERE id > 10;",
    )
    .await;
    let sessions = SessionManager::default();
    sessions
        .open(CONNECTION, &sqlite_params(&path))
        .await
        .unwrap();
    let session = sessions.get(CONNECTION).await.unwrap();
    let table = TableRef {
        database: "main".into(),
        schema: None,
        table: "tuned".into(),
    };
    let analyzed = session
        .maintain_table(&table, Maintenance::Analyze)
        .await
        .unwrap();
    assert_eq!(analyzed.size, None);
    let stats = sessions
        .run_query(
            "t",
            CONNECTION,
            &Default::default(),
            "SELECT count(*) FROM sqlite_stat1 WHERE tbl = 'tuned'",
        )
        .await
        .unwrap();
    assert_eq!(
        stats.status,
        basalt_core::query::OutcomeStatus::Ok,
        "ANALYZE 建了 sqlite_stat1"
    );

    let vacuumed = session
        .maintain_table(&table, Maintenance::Vacuum)
        .await
        .unwrap();
    let size = vacuumed.size.expect("VACUUM 量了大小");
    assert!(size.after * 4 < size.before, "{size:?}");
    assert_eq!(vacuumed.sql, "VACUUM \"main\"");
    let refused = session
        .maintain_table(&table, Maintenance::Check)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), "unsupported");
}
