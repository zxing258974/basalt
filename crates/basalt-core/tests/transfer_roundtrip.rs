//! 导入导出在真库上的往返:四种格式导出,CSV、Excel 导回,逐格一致;SQL 文件执行后也一致。
//! MySQL / MariaDB / PostgreSQL 需要 BASALT_IT 选中目标(见 common);SQLite 总是跑。

mod common;
mod transfer_support;

use basalt_core::session::SessionManager;
use basalt_core::table::TableRef;
use basalt_core::transfer::ExportFormat;
use basalt_core::transfer::import::ImportMode;
use common::*;
use transfer_support::*;

const MYSQL_FIXTURE: &str = r#"
    CREATE DATABASE IF NOT EXISTS basalt_m6 CHARACTER SET utf8mb4;
    DROP TABLE IF EXISTS basalt_m6.src, basalt_m6.dst;
    CREATE TABLE basalt_m6.src (
      id BIGINT UNSIGNED PRIMARY KEY,
      flag TINYINT(1),
      small SMALLINT,
      big BIGINT,
      price DECIMAL(30,10),
      ratio FLOAT,
      score DOUBLE,
      name VARCHAR(50) COLLATE utf8mb4_bin NOT NULL DEFAULT '',
      note TEXT,
      raw VARBINARY(16),
      pic BLOB,
      doc JSON,
      born DATE,
      seen DATETIME(6),
      stamp DATETIME(3),
      span TIME,
      yr YEAR,
      bits BIT(8),
      kind ENUM('a', 'b'),
      perms SET('r', 'w'),
      label VARCHAR(60) AS (CONCAT('#', name)) VIRTUAL
    );
    CREATE TABLE basalt_m6.dst LIKE basalt_m6.src;
    INSERT INTO basalt_m6.src (id, flag, small, big, price, ratio, score, name, note, raw, pic, doc, born, seen, stamp, span, yr, bits, kind, perms) VALUES
      (18446744073709551615, 1, -32768, -9223372036854775808, 12345678901234567890.0123456789,
       0.1, 0.1, '张三 "引号", 逗号', '多行\n文本\r\n第三行', 0x00FF10, X'', '{"a": [1, "二"]}', '2026-10-20',
       '2026-10-20 08:05:09.123456', '2026-10-20 08:05:09.120', '-838:59:59', 2026, b'00000101', 'b', 'r,w'),
      (2, NULL, NULL, NULL, NULL, NULL, NULL, '', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ''),
      (3, 0, 7, 9007199254740993, -0.0000000001, -3.5, 1e300, 'O''Brien \\ 反斜杠', '', X'', 0xDEADBEEF,
       '"字符串"', '1899-01-01', '1000-01-01 00:00:00', '2026-01-01 00:00:00', '25:00:00.5', 1901, b'11111111', 'a', 'w');
"#;

async fn mysql_sessions(target: Target) -> SessionManager {
    let pool = sqlx::MySqlPool::connect(&mysql_url(target))
        .await
        .expect("连接 MySQL 测试容器");
    sqlx::raw_sql(MYSQL_FIXTURE)
        .execute(&pool)
        .await
        .expect("建 M6 测试数据");
    pool.close().await;
    let sessions = SessionManager::default();
    sessions
        .open(CONNECTION, &params(target, TEST_PASSWORD, None))
        .await
        .unwrap();
    sessions
}

fn mysql_table(name: &str) -> TableRef {
    TableRef {
        database: "basalt_m6".into(),
        schema: None,
        table: name.into(),
    }
}

/// SQL 文件交给数据库原样执行,结果要和原表一样。
async fn execute_sql_file(url: &str, mysql: bool, script: &str) {
    if mysql {
        let pool = sqlx::MySqlPool::connect(url).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(script))
            .execute(&pool)
            .await
            .expect("执行导出的 SQL");
        pool.close().await;
    } else {
        let pool = sqlx::PgPool::connect(url).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(script))
            .execute(&pool)
            .await
            .expect("执行导出的 SQL");
        pool.close().await;
    }
}

#[tokio::test]
async fn mysql_family_round_trips_every_format() {
    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let sessions = mysql_sessions(target).await;
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (mysql_table("src"), mysql_table("dst"));
        round_trip(&sessions, &src, &dst, dir.path(), target.name).await;

        // SQL INSERT 文件:清空目标表后执行,和原表一致。
        let path = dir.path().join("script.sql");
        export_table(
            &sessions,
            &src,
            ExportFormat::Sql {
                table: "dst".into(),
                rows_per_statement: 2,
            },
            &path,
        )
        .await;
        let script = format!(
            "DELETE FROM basalt_m6.dst; USE basalt_m6;\n{}",
            std::fs::read_to_string(&path).unwrap()
        );
        execute_sql_file(&mysql_url(target), true, &script).await;
        let (want, got) = (rows(&sessions, &src).await, rows(&sessions, &dst).await);
        assert_eq!(
            want.iter().map(|r| comparable(r)).collect::<Vec<_>>(),
            got.iter().map(|r| comparable(r)).collect::<Vec<_>>(),
            "{} SQL 文件",
            target.name
        );
        sessions.close(CONNECTION).await;
    }
}

const PG_FIXTURE: &str = r#"
    DROP TABLE IF EXISTS public.m6_src, public.m6_dst;
    DROP TYPE IF EXISTS m6_mood;
    CREATE TYPE m6_mood AS ENUM ('sad', 'happy');
    CREATE TABLE public.m6_src (
      id bigint PRIMARY KEY,
      flag boolean,
      small smallint,
      price numeric(40,20),
      huge numeric,
      ratio real,
      score double precision,
      name varchar(50) NOT NULL DEFAULT '',
      code char(4),
      note text,
      uid uuid,
      raw bytea,
      doc json,
      docb jsonb,
      born date,
      seen timestamp(6),
      stamp timestamp(3),
      seen_tz timestamptz,
      at_time time,
      tags text[],
      span interval,
      addr inet,
      feeling m6_mood,
      total numeric GENERATED ALWAYS AS (price * 2) STORED
    );
    CREATE TABLE public.m6_dst (LIKE public.m6_src INCLUDING ALL);
    INSERT INTO public.m6_src (id, flag, small, price, huge, ratio, score, name, code, note, uid, raw,
                               doc, docb, born, seen, stamp, seen_tz, at_time, tags, span, addr, feeling)
    VALUES (9007199254740993, true, -32768, '12345678901234567890.01234567890123456789', 'NaN', 0.1, 'Infinity',
            '张三 "引号", 逗号', 'ab', E'多行\n文本\r\n第三行', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '\x00ff10',
            '{"a": 1,  "b": 2}', '{"b": 2, "a": 1}', '2026-10-20', '2026-10-20 08:05:09.123456',
            '2026-10-20 08:05:09.120', '2026-10-20 00:00:00+00', '24:00:00', '{x,"y z"}', '1 day 02:00:00',
            '10.0.0.1', 'happy'),
           (2, NULL, NULL, NULL, 'Infinity', NULL, 'NaN', '', NULL, '', NULL, '\x', NULL, NULL, 'infinity',
            '-infinity', NULL, NULL, NULL, NULL, NULL, NULL, NULL),
           (3, false, 7, '-0.00000000000000000001', '123456789012345678901234567890123456789.5', -3.5, 1e300,
            'O''Brien \ 反斜杠', NULL, NULL, NULL, '\xdeadbeef', '"字符串"', '[]', '0044-03-15 BC',
            '1899-12-31 23:59:59', '2026-01-01 00:00:00', '2026-06-01 12:30:00+08', '00:00:00.5', '{}', '-3 days',
            '::1', 'sad');
"#;

#[tokio::test]
async fn postgres_round_trips_every_format() {
    if !selected(POSTGRES) {
        return;
    }
    let url = postgres_url(FIXTURE_DB);
    ensure_postgres_database(&postgres_url("postgres"), FIXTURE_DB).await;
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql(PG_FIXTURE)
        .execute(&pool)
        .await
        .expect("建 M6 测试数据");
    pool.close().await;

    let sessions = SessionManager::default();
    sessions
        .open(
            CONNECTION,
            &params(POSTGRES, TEST_PASSWORD, Some(FIXTURE_DB)),
        )
        .await
        .unwrap();
    let table = |name: &str| TableRef {
        database: FIXTURE_DB.into(),
        schema: Some("public".into()),
        table: name.into(),
    };
    let dir = tempfile::tempdir().unwrap();
    let (src, dst) = (table("m6_src"), table("m6_dst"));
    round_trip(&sessions, &src, &dst, dir.path(), "postgres").await;

    let path = dir.path().join("script.sql");
    export_table(
        &sessions,
        &src,
        ExportFormat::Sql {
            table: "m6_dst".into(),
            rows_per_statement: 2,
        },
        &path,
    )
    .await;
    let script = format!(
        "DELETE FROM public.m6_dst;\n{}",
        std::fs::read_to_string(&path).unwrap()
    );
    execute_sql_file(&url, false, &script).await;
    let (want, got) = (rows(&sessions, &src).await, rows(&sessions, &dst).await);
    assert_eq!(
        want.iter().map(|r| comparable(r)).collect::<Vec<_>>(),
        got.iter().map(|r| comparable(r)).collect::<Vec<_>>(),
        "postgres SQL 文件"
    );
    sessions.close(CONNECTION).await;
}

#[tokio::test]
async fn sqlite_round_trips_every_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(
        dir.path(),
        "m6.db",
        "CREATE TABLE src (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, big INTEGER, ratio REAL,
                           price DECIMAL(10,2), born DATE, seen DATETIME, flag BOOLEAN, raw BLOB, anything);
         CREATE TABLE dst (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, big INTEGER, ratio REAL,
                           price DECIMAL(10,2), born DATE, seen DATETIME, flag BOOLEAN, raw BLOB, anything);
         INSERT INTO src VALUES
           (1, '张三, \"引号\"', NULL, 9223372036854775807, 0.1, 12.5, '2026-10-20', '2026-10-20 08:05:09', 1, X'00ff', 'x'),
           (2, '', '', -9223372036854775808, -1e-300, NULL, NULL, NULL, 0, X'', 42),
           (3, '多行' || char(10) || '文本', '尾巴', 0, 1e300, 0, '1899-01-01', '2026-10-20 08:05:09.123', NULL, NULL, NULL);",
    )
    .await;
    let sessions = SessionManager::default();
    sessions
        .open(CONNECTION, &sqlite_params(&path))
        .await
        .unwrap();
    let table = |name: &str| TableRef {
        database: "main".into(),
        schema: None,
        table: name.into(),
    };
    round_trip(
        &sessions,
        &table("src"),
        &table("dst"),
        dir.path(),
        "sqlite",
    )
    .await;
}

/// MySQL 每批提交:遇错即停时,报告里的「已生效」要和表里一致;MyISAM 没有事务,出错前写进去的都在。
#[tokio::test]
async fn mysql_reports_exactly_what_was_committed() {
    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let pool = sqlx::MySqlPool::connect(&mysql_url(target)).await.unwrap();
        sqlx::raw_sql(
            "CREATE DATABASE IF NOT EXISTS basalt_m6 CHARACTER SET utf8mb4;
             DROP TABLE IF EXISTS basalt_m6.batches, basalt_m6.legacy;
             CREATE TABLE basalt_m6.batches (id INT PRIMARY KEY, name VARCHAR(10) NOT NULL);
             CREATE TABLE basalt_m6.legacy (id INT PRIMARY KEY, name VARCHAR(10) NOT NULL) ENGINE=MyISAM;
             INSERT INTO basalt_m6.batches VALUES (99, '原有');
             INSERT INTO basalt_m6.legacy VALUES (99, '原有');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let sessions = SessionManager::default();
        sessions
            .open(CONNECTION, &params(target, TEST_PASSWORD, None))
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        // 第 6 行(第 3 批的第 1 行)和第 3 行重复。
        let path = dir.path().join("dup.csv");
        std::fs::write(&path, "id,name\n1,a\n2,b\n3,c\n4,d\n3,重复\n6,f\n").unwrap();

        // 追加:每批单独提交,前两批留下,原有的 99 不动。
        let report = import_file_with_mode(
            &sessions,
            &mysql_table("batches"),
            &path,
            2,
            ImportMode::Append,
        )
        .await;
        assert_eq!(report.failures[0].row, 6, "{}", target.name);
        assert!(
            report.failures[0].reason.contains("Duplicate"),
            "{}",
            report.failures[0].reason
        );
        assert_eq!(report.affected, 4, "{}：{}", target.name, report.summary);
        assert!(
            report
                .summary
                .contains("前 2 批（到文件第 5 行，共 4 行）已经生效"),
            "{}",
            report.summary
        );
        assert_eq!(
            rows(&sessions, &mysql_table("batches")).await.len(),
            5,
            "{}：原有的 99 和前两批",
            target.name
        );

        // 复制:清空和插入在一个事务里,第 3 批被拒绝时整体撤回,表保持原样。
        let report = import_file(&sessions, &mysql_table("batches"), &path).await;
        assert_eq!(report.affected, 0, "{}：{}", target.name, report.summary);
        assert!(
            report.summary.contains("整个导入已经回滚"),
            "{}",
            report.summary
        );
        assert_eq!(
            rows(&sessions, &mysql_table("batches")).await.len(),
            5,
            "{}：清空也撤回了",
            target.name
        );

        let report = import_file(&sessions, &mysql_table("legacy"), &path).await;
        assert_eq!(report.affected, 4, "{}：{}", target.name, report.summary);
        assert!(report.summary.contains("不支持事务"), "{}", report.summary);
        assert_eq!(
            rows(&sessions, &mysql_table("legacy")).await.len(),
            4,
            "{}",
            target.name
        );
        sessions.close(CONNECTION).await;
    }
}
