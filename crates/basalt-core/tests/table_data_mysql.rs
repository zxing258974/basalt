//! MySQL / MariaDB 的表数据读取与编辑。需要 BASALT_IT 选中目标,见 common。

mod common;

use std::collections::BTreeMap;

use basalt_core::session::Session;
use basalt_core::table::{PageRequest, RowChange, TableRef};
use basalt_core::value::CellValue;
use common::*;
use tokio::sync::OnceCell;

const DB: &str = "basalt_m2";

// 第二行的零日期需要关掉 NO_ZERO_DATE,只对建数据的这条会话生效。
const FIXTURE: &str = r#"
    CREATE DATABASE IF NOT EXISTS basalt_m2 CHARACTER SET utf8mb4;
    DROP TABLE IF EXISTS basalt_m2.types, basalt_m2.people, basalt_m2.log, basalt_m2.legacy;
    SET SESSION sql_mode = '';
    CREATE TABLE basalt_m2.types (
      id BIGINT UNSIGNED PRIMARY KEY,
      flag TINYINT(1),
      small SMALLINT,
      big BIGINT,
      price DECIMAL(30,10),
      ratio FLOAT,
      score DOUBLE,
      name VARCHAR(50) COLLATE utf8mb4_bin,
      note TEXT,
      raw VARBINARY(16),
      doc JSON,
      born DATE,
      seen DATETIME(3),
      span TIME,
      yr YEAR,
      bits BIT(8),
      kind ENUM('a', 'b'),
      shape GEOMETRY
    );
    INSERT INTO basalt_m2.types VALUES
      (18446744073709551615, 2, -32768, -9223372036854775808, 12345678901234567890.0123456789,
       0.1, 0.1, 'Bob', '多行\n文本', 0x00FF10, '{"a": 1}', '2026-10-20',
       '2026-10-20 08:05:09.120', '-838:59:59', 2026, b'00000101', 'b', ST_GeomFromText('POINT(1 1)')),
      (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, '0000-00-00',
       '0000-00-00 00:00:00', NULL, NULL, NULL, NULL, NULL);
    CREATE TABLE basalt_m2.people (
      id INT PRIMARY KEY AUTO_INCREMENT,
      name VARCHAR(50) NOT NULL,
      balance DECIMAL(12,2) DEFAULT 0,
      avatar BLOB,
      label VARCHAR(60) AS (CONCAT('#', name)) VIRTUAL
    );
    INSERT INTO basalt_m2.people (id, name, balance) VALUES (1, 'Ann', 10.50), (2, 'Ben', 20.00);
    CREATE TABLE basalt_m2.log (line TEXT);
    CREATE TABLE basalt_m2.legacy (id INT PRIMARY KEY, v VARCHAR(10)) ENGINE=MyISAM;
    INSERT INTO basalt_m2.legacy VALUES (1, 'a'), (2, 'b');
"#;

static FIXTURES: [OnceCell<()>; 3] = [
    OnceCell::const_new(),
    OnceCell::const_new(),
    OnceCell::const_new(),
];

async fn session_for(index: usize, target: Target) -> Session {
    FIXTURES[index]
        .get_or_init(|| async move {
            let pool = sqlx::MySqlPool::connect(&mysql_url(target))
                .await
                .expect("连接 MySQL 测试容器");
            sqlx::raw_sql(FIXTURE)
                .execute(&pool)
                .await
                .expect("建 M2 测试数据");
            pool.close().await;
        })
        .await;
    Session::open(&params(target, TEST_PASSWORD, None))
        .await
        .unwrap()
}

fn targets() -> impl Iterator<Item = (usize, Target)> {
    MYSQL_FAMILY
        .into_iter()
        .enumerate()
        .filter(|(_, t)| selected(*t))
}

fn table(name: &str) -> TableRef {
    TableRef {
        database: DB.into(),
        schema: None,
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
async fn decodes_every_column_type_without_losing_values() {
    for (index, target) in targets() {
        let session = session_for(index, target).await;
        let page = session
            .read_table(&table("types"), &page(10))
            .await
            .unwrap();
        let column = |name: &str| page.columns.iter().position(|c| c.name == name).unwrap();
        // 按主键升序:第一行是 id = 2,第二行是 id = u64::MAX。
        let zero = &page.rows[0];
        let full = &page.rows[1];
        let cell = |row: &[CellValue], name: &str| row[column(name)].clone();
        let who = target.name;

        assert!(page.columns[0].data_type.contains("unsigned"), "{who}");
        assert_eq!(cell(full, "id"), CellValue::UInt(u64::MAX), "{who}");
        assert_eq!(
            cell(full, "flag"),
            CellValue::Int(2),
            "{who}: TINYINT(1) 不能当布尔值截断"
        );
        assert_eq!(cell(full, "small"), CellValue::Int(-32768), "{who}");
        assert_eq!(cell(full, "big"), CellValue::Int(i64::MIN), "{who}");
        assert_eq!(
            cell(full, "price"),
            CellValue::Decimal("12345678901234567890.0123456789".into()),
            "{who}"
        );
        assert_eq!(cell(full, "ratio"), CellValue::Float(0.1), "{who}");
        assert_eq!(cell(full, "score"), CellValue::Float(0.1), "{who}");
        assert_eq!(
            cell(full, "name"),
            CellValue::Text("Bob".into()),
            "{who}: utf8mb4_bin 列应是文本"
        );
        assert_eq!(
            cell(full, "note"),
            CellValue::Text("多行\n文本".into()),
            "{who}"
        );
        assert_eq!(
            cell(full, "raw"),
            CellValue::Bytes(vec![0x00, 0xff, 0x10]),
            "{who}"
        );
        // MariaDB 的 JSON 是 LONGTEXT 的别名。
        let doc = cell(full, "doc");
        assert!(
            matches!(&doc, CellValue::Json(t) | CellValue::Text(t) if t.replace(' ', "") == "{\"a\":1}"),
            "{who}: {doc:?}"
        );
        assert_eq!(
            cell(full, "born"),
            CellValue::Date("2026-10-20".into()),
            "{who}"
        );
        assert_eq!(
            cell(full, "seen"),
            CellValue::DateTime("2026-10-20 08:05:09.12".into()),
            "{who}"
        );
        assert_eq!(
            cell(full, "span"),
            CellValue::Time("-838:59:59".into()),
            "{who}"
        );
        assert_eq!(cell(full, "yr"), CellValue::UInt(2026), "{who}");
        assert_eq!(cell(full, "bits"), CellValue::UInt(5), "{who}");
        assert_eq!(cell(full, "kind"), CellValue::Text("b".into()), "{who}");
        assert_eq!(
            cell(full, "shape"),
            CellValue::unsupported("GEOMETRY"),
            "{who}"
        );

        assert_eq!(
            cell(zero, "born"),
            CellValue::Date("0000-00-00".into()),
            "{who}"
        );
        assert_eq!(
            cell(zero, "seen"),
            CellValue::DateTime("0000-00-00 00:00:00".into()),
            "{who}"
        );
        assert_eq!(cell(zero, "flag"), CellValue::Null, "{who}");
        session.close().await;
    }
}

#[tokio::test]
async fn edits_commit_in_one_transaction_and_roll_back_together() {
    for (index, target) in targets() {
        let session = session_for(index, target).await;
        let who = target.name;
        let people = table("people");
        let read = || async { session.read_table(&people, &page(10)).await.unwrap().rows };

        let update = [RowChange::Update {
            key: key(1),
            values: values(&[
                ("name", Some("Anne")),
                ("balance", Some("11.25")),
                ("avatar", Some("0x0a0b")),
            ]),
        }];
        assert_eq!(
            session.preview_changes(&people, &update).await.unwrap(),
            [
                "UPDATE `basalt_m2`.`people` SET `name` = 'Anne', `balance` = '11.25', `avatar` = X'0a0b' WHERE `id` = 1"
            ],
            "{who}"
        );
        let outcome = session.apply_changes(&people, &update, None).await.unwrap();
        assert_eq!((outcome.statements, outcome.affected_rows), (1, 1), "{who}");
        let rows = read().await;
        assert_eq!(rows[0][1], CellValue::Text("Anne".into()), "{who}");
        assert_eq!(rows[0][2], CellValue::Decimal("11.25".into()), "{who}");
        assert_eq!(rows[0][3], CellValue::Bytes(vec![0x0a, 0x0b]), "{who}");
        assert_eq!(
            rows[0][4],
            CellValue::Text("#Anne".into()),
            "{who}: 生成列随之更新"
        );

        // 值没变的 UPDATE 也算命中一行(连接开了 FOUND_ROWS),不能误报。
        session.apply_changes(&people, &update, None).await.unwrap();

        let insert = [RowChange::Insert {
            values: values(&[("name", Some("Cat"))]),
        }];
        session.apply_changes(&people, &insert, None).await.unwrap();
        let rows = read().await;
        assert_eq!(rows.len(), 3, "{who}");
        assert_eq!(
            rows[2][2],
            CellValue::Decimal("0.00".into()),
            "{who}: 没填的列用默认值"
        );
        let CellValue::Int(new_id) = rows[2][0] else {
            panic!("{who}: 自增主键应是整数");
        };
        session
            .apply_changes(&people, &[RowChange::Delete { key: key(new_id) }], None)
            .await
            .unwrap();

        let missing = [
            RowChange::Update {
                key: key(1),
                values: values(&[("name", Some("X"))]),
            },
            RowChange::Delete { key: key(99) },
        ];
        let error = session
            .apply_changes(&people, &missing, None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "row_mismatch", "{who}: {error}");

        let duplicate = [
            RowChange::Update {
                key: key(1),
                values: values(&[("name", Some("Y"))]),
            },
            RowChange::Insert {
                values: values(&[("id", Some("2")), ("name", Some("重复"))]),
            },
        ];
        let error = session
            .apply_changes(&people, &duplicate, None)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("第 2 条语句执行失败"),
            "{who}: {error}"
        );
        assert_eq!(
            read().await[0][1],
            CellValue::Text("Anne".into()),
            "{who}: 失败的批次不应留下修改"
        );

        for bad in [
            values(&[("balance", Some("abc"))]),
            values(&[("label", Some("x"))]),
        ] {
            let change = [RowChange::Update {
                key: key(1),
                values: bad,
            }];
            let error = session
                .apply_changes(&people, &change, None)
                .await
                .unwrap_err();
            assert_eq!(error.code(), "invalid_edit", "{who}: {error}");
        }

        let log = session.read_table(&table("log"), &page(10)).await.unwrap();
        assert!(log.read_only_reason.is_some(), "{who}");
        session.close().await;
    }
}

#[tokio::test]
async fn filters_and_sorts_run_on_the_server() {
    for (index, target) in targets() {
        let session = session_for(index, target).await;
        let who = target.name;
        let filtered = PageRequest {
            filter: Some("name = 'Bob' -- 只看 Bob".into()),
            ..page(10)
        };
        let rows = session
            .read_table(&table("types"), &filtered)
            .await
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 1, "{who}");

        let one = session.read_table(&table("types"), &page(1)).await.unwrap();
        assert!(one.has_more, "{who}");
        assert!(
            one.sql.ends_with("ORDER BY `id` LIMIT 2 OFFSET 0"),
            "{who}: {}",
            one.sql
        );

        // `||` 必须是 MySQL 默认的逻辑或,不能被 sqlx 的 PIPES_AS_CONCAT 改成字符串拼接。
        // 拼接语义下是 id = (2 || id) = …,两行都不命中。
        let either = PageRequest {
            filter: Some("id = 2 || id = 18446744073709551615".into()),
            ..page(10)
        };
        let rows = session
            .read_table(&table("types"), &either)
            .await
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 2, "{who}: || 应是逻辑或");

        let injected = PageRequest {
            filter: Some("1 = 1; DROP TABLE basalt_m2.types".into()),
            ..page(10)
        };
        let error = session
            .read_table(&table("types"), &injected)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_filter", "{who}");
        session.close().await;
    }
}

#[tokio::test]
async fn tables_without_transactions_take_one_statement_at_a_time() {
    for (index, target) in targets() {
        let session = session_for(index, target).await;
        let who = target.name;
        let legacy = table("legacy");
        let update = |id: i64, v: &str| RowChange::Update {
            key: key(id),
            values: values(&[("v", Some(v))]),
        };
        // MyISAM 不能回滚:一批两条时,第二条失败也撤不回第一条,所以干脆不让提交。
        let error = session
            .preview_changes(&legacy, &[update(1, "x"), update(2, "y")])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_edit", "{who}");
        assert!(error.to_string().contains("MyISAM"), "{who}: {error}");

        session
            .apply_changes(&legacy, &[update(1, "x")], None)
            .await
            .unwrap();
        let rows = session.read_table(&legacy, &page(10)).await.unwrap().rows;
        assert_eq!(rows[0][1], CellValue::Text("x".into()), "{who}");
        session.close().await;
    }
}
