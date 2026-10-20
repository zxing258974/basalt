//! SQLite 的表数据读取与编辑,真实文件、无需容器。

mod common;

use std::collections::BTreeMap;

use basalt_core::session::Session;
use basalt_core::table::{PageRequest, RowChange, SortSpec, TableRef};
use basalt_core::value::CellValue;
use common::*;

const SCRIPT: &str = "
    CREATE TABLE items (
      id INTEGER PRIMARY KEY,
      title TEXT NOT NULL,
      price REAL,
      qty INTEGER DEFAULT 1,
      blob BLOB,
      flag BOOLEAN,
      total INTEGER GENERATED ALWAYS AS (qty * 2) VIRTUAL
    );
    INSERT INTO items (id, title, price, qty, blob, flag) VALUES
      (1, '苹果', 3.5, 2, x'00ff', 1),
      (2, 'O''Brien', NULL, 5, NULL, 0),
      (3, '多行\n文本', 1e300, 1, 'text in blob', NULL);
    CREATE TABLE log (line TEXT);
    INSERT INTO log VALUES ('只读');
    CREATE TABLE codes (code TEXT PRIMARY KEY, note TEXT);
    INSERT INTO codes VALUES (NULL, '主键是 NULL');
";

fn items() -> TableRef {
    TableRef {
        database: "main".into(),
        schema: None,
        table: "items".into(),
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

async fn open() -> (tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "data.db", SCRIPT).await;
    let session = Session::open(&sqlite_params(&path)).await.unwrap();
    (dir, session)
}

#[tokio::test]
async fn reads_columns_and_decodes_each_storage_class() {
    let (_dir, session) = open().await;
    let page = session.read_table(&items(), &page(100)).await.unwrap();

    let names: Vec<_> = page.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["id", "title", "price", "qty", "blob", "flag", "total"]
    );
    let id = &page.columns[0];
    assert!(
        id.is_primary_key && id.has_default,
        "INTEGER PRIMARY KEY 是 rowid 别名"
    );
    assert!(!page.columns[1].nullable);
    assert!(page.columns[3].has_default);
    assert!(page.columns[6].is_generated);
    assert_eq!(page.read_only_reason, None);
    assert!(!page.has_more);

    assert_eq!(
        page.rows[0],
        [
            CellValue::Int(1),
            CellValue::Text("苹果".into()),
            CellValue::Float(3.5),
            CellValue::Int(2),
            CellValue::Bytes(vec![0x00, 0xff]),
            CellValue::Int(1),
            CellValue::Int(4),
        ]
    );
    assert_eq!(page.rows[1][2], CellValue::Null);
    // 动态类型:BLOB 列里存的文本仍按文本解码。
    assert_eq!(page.rows[2][4], CellValue::Text("text in blob".into()));
    assert_eq!(page.rows[2][1], CellValue::Text("多行\n文本".into()));
    session.close().await;
}

#[tokio::test]
async fn pages_sorts_and_filters() {
    let (_dir, session) = open().await;
    let first = session.read_table(&items(), &page(2)).await.unwrap();
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);

    let rest = PageRequest {
        offset: 2,
        ..page(2)
    };
    let second = session.read_table(&items(), &rest).await.unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(!second.has_more);
    assert_eq!(second.rows[0][0], CellValue::Int(3));

    let by_qty = PageRequest {
        sort: Some(SortSpec {
            column: "qty".into(),
            descending: true,
        }),
        ..page(10)
    };
    let sorted = session.read_table(&items(), &by_qty).await.unwrap();
    let ids: Vec<_> = sorted.rows.iter().map(|r| r[0].clone()).collect();
    assert_eq!(
        ids,
        [CellValue::Int(2), CellValue::Int(1), CellValue::Int(3)]
    );
    assert!(
        sorted.sql.contains("ORDER BY \"qty\" DESC, \"id\""),
        "{}",
        sorted.sql
    );

    let filtered = PageRequest {
        filter: Some("price IS NULL OR title LIKE '苹%'".into()),
        ..page(10)
    };
    let rows = session.read_table(&items(), &filtered).await.unwrap().rows;
    assert_eq!(rows.len(), 2);

    let injected = PageRequest {
        filter: Some("1 = 1; DELETE FROM items".into()),
        ..page(10)
    };
    let error = session.read_table(&items(), &injected).await.unwrap_err();
    assert_eq!(error.code(), "invalid_filter");
    assert_eq!(
        session
            .read_table(&items(), &page(10))
            .await
            .unwrap()
            .rows
            .len(),
        3
    );

    let broken = PageRequest {
        filter: Some("no_such_column = 1".into()),
        ..page(10)
    };
    let error = session.read_table(&items(), &broken).await.unwrap_err();
    assert_eq!(error.code(), "database", "{error}");
    session.close().await;
}

#[tokio::test]
async fn previews_and_applies_update_insert_delete() {
    let (_dir, session) = open().await;
    let changes = [
        RowChange::Update {
            key: key(1),
            values: values(&[
                ("title", Some("梨")),
                ("price", None),
                ("blob", Some("0x0102")),
            ]),
        },
        RowChange::Insert {
            values: values(&[("title", Some("新行")), ("qty", Some("7"))]),
        },
        RowChange::Delete { key: key(2) },
    ];
    let preview = session.preview_changes(&items(), &changes).await.unwrap();
    assert_eq!(
        preview,
        [
            "DELETE FROM \"main\".\"items\" WHERE \"id\" = 2",
            "UPDATE \"main\".\"items\" SET \"title\" = '梨', \"price\" = NULL, \"blob\" = X'0102' WHERE \"id\" = 1",
            "INSERT INTO \"main\".\"items\" (\"title\", \"qty\") VALUES ('新行', '7')",
        ]
    );

    let outcome = session
        .apply_changes(&items(), &changes, None)
        .await
        .unwrap();
    assert_eq!((outcome.statements, outcome.affected_rows), (3, 3));

    let rows = session.read_table(&items(), &page(10)).await.unwrap().rows;
    let ids: Vec<_> = rows.iter().map(|r| r[0].clone()).collect();
    assert_eq!(
        ids,
        [CellValue::Int(1), CellValue::Int(3), CellValue::Int(4)]
    );
    assert_eq!(rows[0][1], CellValue::Text("梨".into()));
    assert_eq!(rows[0][2], CellValue::Null);
    assert_eq!(rows[0][4], CellValue::Bytes(vec![1, 2]));
    // '7' 按 INTEGER 亲和性存成了整数,生成列随之更新。
    assert_eq!(rows[2][3], CellValue::Int(7));
    assert_eq!(rows[2][6], CellValue::Int(14));
    session.close().await;
}

#[tokio::test]
async fn any_failure_rolls_back_the_whole_batch() {
    let (_dir, session) = open().await;
    let missing_row = [
        RowChange::Update {
            key: key(1),
            values: values(&[("title", Some("改了"))]),
        },
        RowChange::Delete { key: key(99) },
    ];
    let error = session
        .apply_changes(&items(), &missing_row, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "row_mismatch");
    assert!(error.to_string().contains("影响了 0 行"), "{error}");

    let constraint = [
        RowChange::Update {
            key: key(1),
            values: values(&[("title", Some("改了"))]),
        },
        RowChange::Insert {
            values: values(&[("id", Some("2")), ("title", Some("主键重复"))]),
        },
    ];
    let error = session
        .apply_changes(&items(), &constraint, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "database");
    assert!(
        error
            .to_string()
            .contains("第 2 条语句执行失败，所有修改都已回滚"),
        "{error}"
    );

    let rows = session.read_table(&items(), &page(10)).await.unwrap().rows;
    assert_eq!(
        rows[0][1],
        CellValue::Text("苹果".into()),
        "两次失败都不应留下修改"
    );
    session.close().await;
}

#[tokio::test]
async fn tables_without_primary_key_are_read_only_and_null_keys_still_locate() {
    let (_dir, session) = open().await;
    let log = TableRef {
        table: "log".into(),
        ..items()
    };
    let page_of_log = session.read_table(&log, &page(10)).await.unwrap();
    assert!(page_of_log.read_only_reason.unwrap().contains("没有主键"));
    let error = session
        .apply_changes(
            &log,
            &[RowChange::Insert {
                values: values(&[("line", Some("x"))]),
            }],
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "read_only");

    let codes = TableRef {
        table: "codes".into(),
        ..items()
    };
    let change = RowChange::Update {
        key: BTreeMap::from([("code".to_owned(), CellValue::Null)]),
        values: values(&[("note", Some("找到了"))]),
    };
    session
        .apply_changes(&codes, &[change], None)
        .await
        .unwrap();
    let rows = session.read_table(&codes, &page(10)).await.unwrap().rows;
    assert_eq!(rows[0][1], CellValue::Text("找到了".into()));

    let missing = TableRef {
        table: "nope".into(),
        ..items()
    };
    let error = session.read_table(&missing, &page(10)).await.unwrap_err();
    assert!(error.to_string().contains("找不到表"), "{error}");
    session.close().await;
}

#[tokio::test]
async fn apply_refuses_sql_that_differs_from_the_confirmed_preview() {
    let (_dir, session) = open().await;
    let changes = [RowChange::Update {
        key: key(1),
        values: values(&[("title", Some("梨"))]),
    }];
    let preview = session.preview_changes(&items(), &changes).await.unwrap();
    let stale =
        vec!["UPDATE \"main\".\"items\" SET \"title\" = '别的' WHERE \"id\" = 1".to_owned()];
    let error = session
        .apply_changes(&items(), &changes, Some(&stale))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_edit");
    assert!(error.to_string().contains("重新预览"), "{error}");

    let outcome = session
        .apply_changes(&items(), &changes, Some(&preview))
        .await
        .unwrap();
    assert_eq!(outcome.affected_rows, 1);
    session.close().await;
}
