//! PostgreSQL 的表数据读取与编辑。需要 BASALT_IT 选中 postgres,见 common。

mod common;

use std::collections::BTreeMap;

use basalt_core::session::Session;
use basalt_core::table::{PageRequest, RowChange, SortSpec, TableRef};
use basalt_core::value::CellValue;
use chrono::{Local, TimeZone, Utc};
use common::*;
use tokio::sync::OnceCell;

const DB: &str = "basalt_m2";

const FIXTURE: &str = r#"
    DROP TABLE IF EXISTS public.types, public.people, public.events, public.log;
    DROP TYPE IF EXISTS mood;
    DROP DOMAIN IF EXISTS positive_int;
    CREATE TYPE mood AS ENUM ('sad', 'happy');
    CREATE DOMAIN positive_int AS integer CHECK (VALUE > 0);
    CREATE TABLE public.types (
      id bigint PRIMARY KEY,
      flag boolean,
      small smallint,
      price numeric(40,20),
      huge numeric,
      ratio real,
      score double precision,
      name varchar(50),
      code char(4),
      note text,
      uid uuid,
      raw bytea,
      doc json,
      docb jsonb,
      born date,
      seen timestamp(3),
      seen_tz timestamptz,
      at_time time,
      tags text[],
      span interval,
      addr inet,
      feeling mood,
      stock positive_int,
      total numeric GENERATED ALWAYS AS (price * 2) STORED
    );
    INSERT INTO public.types (id, flag, small, price, huge, ratio, score, name, code, note, uid, raw,
                              doc, docb, born, seen, seen_tz, at_time, tags, span, addr, feeling, stock)
    VALUES (9007199254740993, true, -32768, '12345678901234567890.01234567890123456789', 'NaN', 0.1, 0.1,
            'Bob', 'ab', E'多行\n文本', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '\x00ff10',
            '{"a": 1,  "b": 2}', '{"b": 2, "a": 1}', '2026-10-20', '2026-10-20 08:05:09.120',
            '2026-10-20 00:00:00+00', '24:00:00', '{x,"y z"}', '1 day 02:00:00', '10.0.0.1', 'happy', 5),
           (2, NULL, NULL, NULL, 'Infinity', NULL, NULL, NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, 'infinity', '-infinity', NULL, NULL, NULL, NULL, NULL, NULL, NULL);
    CREATE TABLE public.people (
      id serial PRIMARY KEY,
      name text NOT NULL,
      tags text[],
      span interval,
      feeling mood,
      photo bytea,
      stock positive_int,
      code char(4)
    );
    INSERT INTO public.people (id, name, tags, span, feeling, code)
    VALUES (1, 'Ann', '{a}', '1 hour', 'sad', 'ab'), (2, 'Ben', NULL, NULL, NULL, NULL);
    SELECT setval('people_id_seq', 2);
    CREATE TABLE public.events (at timestamptz, code char(4), note text, PRIMARY KEY (at, code));
    INSERT INTO public.events VALUES ('2026-10-20 01:02:03.456789+00', 'ab', 'x');
    CREATE TABLE public.log (line text);
"#;

static FIXTURE_READY: OnceCell<()> = OnceCell::const_new();

/// 没选中 postgres 时返回 `None`,测试直接跳过。
async fn session() -> Option<Session> {
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
                sqlx::raw_sql("CREATE DATABASE basalt_m2")
                    .execute(&admin)
                    .await
                    .unwrap();
            }
            admin.close().await;
            let db = sqlx::PgPool::connect(&postgres_url(DB)).await.unwrap();
            sqlx::raw_sql(FIXTURE)
                .execute(&db)
                .await
                .expect("建 M2 测试数据");
            db.close().await;
        })
        .await;
    Some(
        Session::open(&params(POSTGRES, TEST_PASSWORD, None))
            .await
            .unwrap(),
    )
}

fn table(name: &str) -> TableRef {
    TableRef {
        database: DB.into(),
        schema: Some("public".into()),
        table: name.into(),
    }
}

fn page(limit: u32) -> PageRequest {
    PageRequest {
        offset: 0,
        limit,
        sort: None,
        filter: None,
    }
}

fn key(id: i64) -> BTreeMap<String, CellValue> {
    BTreeMap::from([("id".to_owned(), CellValue::Int(id))])
}

fn values(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.map(str::to_owned)))
        .collect()
}

#[tokio::test]
async fn decodes_native_types_and_reads_the_rest_as_text() {
    let Some(session) = session().await else {
        return;
    };
    let page = session
        .read_table(&table("types"), &page(10))
        .await
        .unwrap();
    let position = |name: &str| page.columns.iter().position(|c| c.name == name).unwrap();
    let infinite = &page.rows[0];
    let full = &page.rows[1];
    let cell = |row: &[CellValue], name: &str| row[position(name)].clone();

    let local = Local.from_utc_datetime(
        &Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0)
            .unwrap()
            .naive_utc(),
    );
    let expected_tz = local.format("%Y-%m-%d %H:%M:%S%:z").to_string();
    let expected = [
        ("id", CellValue::Int(9_007_199_254_740_993)),
        ("flag", CellValue::Bool(true)),
        ("small", CellValue::Int(-32768)),
        (
            "price",
            CellValue::Decimal("12345678901234567890.01234567890123456789".into()),
        ),
        ("huge", CellValue::Decimal("NaN".into())),
        ("ratio", CellValue::Float(0.1)),
        ("score", CellValue::Float(0.1)),
        ("name", CellValue::Text("Bob".into())),
        ("code", CellValue::Text("ab  ".into())),
        ("note", CellValue::Text("多行\n文本".into())),
        (
            "uid",
            CellValue::Text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
        ),
        ("raw", CellValue::Bytes(vec![0x00, 0xff, 0x10])),
        ("doc", CellValue::Json(r#"{"a": 1,  "b": 2}"#.into())),
        ("docb", CellValue::Json(r#"{"a": 1, "b": 2}"#.into())),
        ("born", CellValue::Date("2026-10-20".into())),
        ("seen", CellValue::DateTime("2026-10-20 08:05:09.12".into())),
        ("seen_tz", CellValue::DateTime(expected_tz)),
        ("at_time", CellValue::Time("24:00:00".into())),
        ("tags", CellValue::Text(r#"{x,"y z"}"#.into())),
        ("span", CellValue::Text("1 day 02:00:00".into())),
        // inet 转 text 走专门的函数,总带掩码;写回 '10.0.0.1/32'::inet 一样解析。
        ("addr", CellValue::Text("10.0.0.1/32".into())),
        ("feeling", CellValue::Text("happy".into())),
        ("stock", CellValue::Int(5)),
        (
            "total",
            CellValue::Decimal("24691357802469135780.02469135780246913578".into()),
        ),
    ];
    for (name, value) in expected {
        assert_eq!(cell(full, name), value, "{name}");
    }
    assert_eq!(
        cell(infinite, "huge"),
        CellValue::Decimal("Infinity".into())
    );
    assert_eq!(cell(infinite, "born"), CellValue::Date("infinity".into()));
    assert_eq!(
        cell(infinite, "seen"),
        CellValue::DateTime("-infinity".into())
    );
    assert_eq!(cell(infinite, "flag"), CellValue::Null);

    let column = |name: &str| &page.columns[position(name)];
    assert_eq!(column("code").data_type, "character(4)");
    assert_eq!(column("tags").data_type, "text[]");
    assert!(column("total").is_generated);
    assert!(column("id").is_primary_key && !column("id").nullable);
    assert!(
        page.sql.contains("\"tags\"::text AS \"tags\""),
        "{}",
        page.sql
    );
    session.close().await;
}

#[tokio::test]
async fn edits_go_through_casts_and_roll_back_together() {
    let Some(session) = session().await else {
        return;
    };
    let people = table("people");
    let read = || async { session.read_table(&people, &page(10)).await.unwrap().rows };

    let update = [RowChange::Update {
        key: key(1),
        values: values(&[
            ("name", Some("Anne")),
            ("tags", Some("{a,b}")),
            ("span", Some("3 hours")),
            ("feeling", Some("happy")),
            ("photo", Some("0x0a0b")),
            ("stock", Some("7")),
        ]),
    }];
    assert_eq!(
        session.preview_changes(&people, &update).await.unwrap(),
        [
            "UPDATE \"public\".\"people\" SET \"name\" = 'Anne', \"tags\" = '{a,b}'::text[], \
             \"span\" = '3 hours'::interval, \"feeling\" = 'happy'::mood, \"photo\" = '\\x0a0b'::bytea, \
             \"stock\" = '7'::positive_int WHERE \"id\" = '1'::integer"
        ]
    );
    session.apply_changes(&people, &update, None).await.unwrap();
    let row = read().await.remove(0);
    assert_eq!(row[1], CellValue::Text("Anne".into()));
    assert_eq!(row[2], CellValue::Text("{a,b}".into()));
    assert_eq!(row[3], CellValue::Text("03:00:00".into()));
    assert_eq!(row[4], CellValue::Text("happy".into()));
    assert_eq!(row[5], CellValue::Bytes(vec![0x0a, 0x0b]));
    assert_eq!(row[6], CellValue::Int(7));

    // 域约束失败:数据库报错,整批回滚。
    let violation = [
        RowChange::Update {
            key: key(1),
            values: values(&[("name", Some("X"))]),
        },
        RowChange::Update {
            key: key(2),
            values: values(&[("stock", Some("-1"))]),
        },
    ];
    let error = session
        .apply_changes(&people, &violation, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("第 2 条语句执行失败"), "{error}");
    let missing = [
        RowChange::Update {
            key: key(1),
            values: values(&[("name", Some("Y"))]),
        },
        RowChange::Delete { key: key(99) },
    ];
    assert_eq!(
        session
            .apply_changes(&people, &missing, None)
            .await
            .unwrap_err()
            .code(),
        "row_mismatch"
    );
    assert_eq!(read().await[0][1], CellValue::Text("Anne".into()));

    // 自增列不填,用序列的默认值;再按读回来的主键删掉。
    session
        .apply_changes(
            &people,
            &[RowChange::Insert {
                values: values(&[("name", Some("Cat"))]),
            }],
            None,
        )
        .await
        .unwrap();
    let rows = read().await;
    assert_eq!(rows[2][0], CellValue::Int(3));
    session
        .apply_changes(&people, &[RowChange::Delete { key: key(3) }], None)
        .await
        .unwrap();
    assert_eq!(read().await.len(), 2);
    session.close().await;
}

#[tokio::test]
async fn keys_read_back_locate_the_same_row_exactly() {
    let Some(session) = session().await else {
        return;
    };
    // 主键是带微秒的 timestamptz 加补空格的 char(4):读出来的文本写回去必须精确命中。
    let events = table("events");
    let row = session
        .read_table(&events, &page(10))
        .await
        .unwrap()
        .rows
        .remove(0);
    let key = BTreeMap::from([
        ("at".to_owned(), row[0].clone()),
        ("code".to_owned(), row[1].clone()),
    ]);
    let change = RowChange::Update {
        key,
        values: values(&[("note", Some("命中了"))]),
    };
    let outcome = session
        .apply_changes(&events, &[change], None)
        .await
        .unwrap();
    assert_eq!(outcome.affected_rows, 1);

    let log = session.read_table(&table("log"), &page(10)).await.unwrap();
    assert!(log.read_only_reason.is_some());

    let sorted = PageRequest {
        sort: Some(SortSpec {
            column: "tags".into(),
            descending: false,
        }),
        filter: Some("tags IS NOT NULL OR id = 2".into()),
        ..page(10)
    };
    let rows = session
        .read_table(&table("types"), &sorted)
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 2, "按转成文本读的列排序也要能执行");
    session.close().await;
}
