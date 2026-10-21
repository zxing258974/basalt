//! 五种导入模式在真的 SQLite 上的行为;MySQL、PostgreSQL 的在集成测试 transfer_modes.rs 里。

use super::*;
use crate::query::OutcomeStatus;
use crate::transfer::progress::ignore;
use crate::transfer::test_support::{open_sqlite, table_ref};
use crate::value::CellValue;

const SCHEMA: &str = "
    CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, code TEXT);
    CREATE UNIQUE INDEX people_code ON people (code);
    INSERT INTO people VALUES (1, '张三', '老员工', 'A'), (2, '李四', NULL, 'B'), (3, '王五', '', 'C');
";

fn options(mode: ImportMode, keys: &[&str], on_error: OnError) -> ImportOptions {
    ImportOptions {
        on_error,
        batch_size: 2,
        mode,
        key_columns: keys.iter().map(|k| (*k).to_owned()).collect(),
        binary: BinaryEncoding::Hex,
    }
}

/// 把 `csv` 写成文件,按列名自动映射后导入 people。
async fn import_csv(
    sessions: &SessionManager,
    dir: &tempfile::TempDir,
    csv: &str,
    options: ImportOptions,
) -> CoreResult<ImportReport> {
    let path = dir.path().join("in.csv");
    std::fs::write(&path, csv).unwrap();
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(sessions, "c1", &table_ref("people"), spec.clone()).await?;
    let request = ImportRequest {
        connection_id: "c1".into(),
        table: table_ref("people"),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options,
    };
    run_import(sessions, "job", &request, &CancelFlag::default(), &ignore).await
}

/// 整张表按 id 排好,每行拼成「id 名字 备注 代码」,NULL 写成 ∅。
async fn table(sessions: &SessionManager) -> Vec<String> {
    let outcome = sessions
        .run_query(
            "check",
            "c1",
            &QueryScope::default(),
            "SELECT id, name, note, code FROM people ORDER BY id",
        )
        .await
        .unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Ok, "{:?}", outcome.message);
    outcome.result_sets[0]
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    CellValue::Null => "∅".to_owned(),
                    CellValue::Int(n) => n.to_string(),
                    CellValue::Text(t) => t.clone(),
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[tokio::test]
async fn preview_lists_the_primary_key_first_then_unique_indexes() {
    let (dir, sessions) = open_sqlite(SCHEMA).await;
    let path = dir.path().join("k.csv");
    std::fs::write(&path, "id\n1\n").unwrap();
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(&sessions, "c1", &table_ref("people"), spec)
        .await
        .unwrap();
    assert_eq!(
        preview.unique_keys,
        [
            UniqueKey {
                name: None,
                columns: vec!["id".into()]
            },
            UniqueKey {
                name: Some("people_code".into()),
                columns: vec!["code".into()]
            },
        ]
    );
}

#[tokio::test]
async fn update_changes_only_mapped_columns_and_counts_rows_it_could_not_find() {
    let (dir, sessions) = open_sqlite(SCHEMA).await;
    let csv = "id,name\n1,张三丰\n9,查无此人\n3,王五五\n";
    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Update, &["id"], OnError::Stop),
    )
    .await
    .unwrap();
    assert_eq!(report.status, ImportStatus::Completed, "{}", report.summary);
    assert_eq!(
        (report.affected, report.unmatched, report.failed),
        (2, 1, 0)
    );
    assert_eq!(
        report.summary,
        "更新完成：改了表里 2 行，文件里 1 行在表里没找到对应的行。"
    );
    assert_eq!(
        table(&sessions).await,
        ["1 张三丰 老员工 A", "2 李四 ∅ B", "3 王五五  C"],
        "备注和代码没映射,保持原样"
    );
}

#[tokio::test]
async fn upsert_updates_existing_rows_and_inserts_new_ones() {
    let (dir, sessions) = open_sqlite(SCHEMA).await;
    let csv = "id,name,code\n2,李四四,B\n4,赵六,D\n5,钱七,E\n";
    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Upsert, &["id"], OnError::Stop),
    )
    .await
    .unwrap();
    assert_eq!(
        (report.status, report.affected),
        (ImportStatus::Completed, 3),
        "{}",
        report.summary
    );
    assert_eq!(
        table(&sessions).await,
        [
            "1 张三 老员工 A",
            "2 李四四 ∅ B",
            "3 王五  C",
            "4 赵六 ∅ D",
            "5 钱七 ∅ E"
        ]
    );

    // 按唯一索引匹配;同一批里同一个键出现两次,后面的说了算。
    let csv = "code,name\nA,张三（改）\nF,孙八\nF,孙八八\n";
    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Upsert, &["code"], OnError::Stop),
    )
    .await
    .unwrap();
    assert_eq!(report.status, ImportStatus::Completed, "{}", report.summary);
    let rows = table(&sessions).await;
    assert_eq!(rows[0], "1 张三（改） 老员工 A");
    assert_eq!(rows.last().unwrap(), "6 孙八八 ∅ F");
}

#[tokio::test]
async fn upsert_with_only_the_key_mapped_leaves_existing_rows_alone() {
    let (dir, sessions) = open_sqlite(
        "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, note TEXT, code TEXT);
         INSERT INTO people VALUES (1, '张三', NULL, NULL);",
    )
    .await;
    let report = import_csv(
        &sessions,
        &dir,
        "id\n1\n2\n",
        options(ImportMode::Upsert, &["id"], OnError::Stop),
    )
    .await
    .unwrap();
    assert_eq!(report.status, ImportStatus::Completed, "{}", report.summary);
    assert_eq!(table(&sessions).await, ["1 张三 ∅ ∅", "2 ∅ ∅ ∅"]);
}

#[tokio::test]
async fn delete_uses_only_the_key_columns() {
    let (dir, sessions) = open_sqlite(SCHEMA).await;
    // 名字这一列有个空值,但删除用不到它,不会因此失败。
    let csv = "id,name\n2,\n7,不存在\n";
    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Delete, &["id"], OnError::Stop),
    )
    .await
    .unwrap();
    assert_eq!(
        (report.affected, report.unmatched, report.failed),
        (1, 1, 0),
        "{}",
        report.summary
    );
    assert_eq!(
        report.summary,
        "删除完成：删掉表里 1 行，文件里 1 行在表里没找到对应的行。"
    );
    assert_eq!(table(&sessions).await, ["1 张三 老员工 A", "3 王五  C"]);
}

#[tokio::test]
async fn rejected_updates_roll_back_or_are_skipped() {
    let (dir, sessions) = open_sqlite(SCHEMA).await;
    // 第 2 行把代码改成和别人重复,违反唯一索引。
    let csv = "id,code\n1,X\n2,C\n3,Z\n";
    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Update, &["id"], OnError::Stop),
    )
    .await
    .unwrap();
    assert_eq!(report.status, ImportStatus::Stopped, "{}", report.summary);
    assert_eq!(report.affected, 0);
    assert_eq!(table(&sessions).await[0], "1 张三 老员工 A", "整体回滚");

    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Update, &["id"], OnError::Skip),
    )
    .await
    .unwrap();
    assert_eq!(
        (report.status, report.affected, report.failed),
        (ImportStatus::Completed, 2, 1)
    );
    assert_eq!(report.failures[0].row, 3, "CSV 的行号把列名行算作第 1 行");
    assert_eq!(
        table(&sessions).await,
        ["1 张三 老员工 X", "2 李四 ∅ B", "3 王五  Z"]
    );
}

#[tokio::test]
async fn keys_are_checked_before_anything_is_written() {
    let (dir, sessions) = open_sqlite(SCHEMA).await;
    let csv = "id,name\n1,甲\n";
    let cases = [
        (
            options(ImportMode::Update, &[], OnError::Stop),
            "要选匹配键",
        ),
        (
            options(ImportMode::Delete, &["code"], OnError::Stop),
            "没有选来源列",
        ),
        (
            options(ImportMode::Upsert, &["name"], OnError::Stop),
            "只能按主键或一个唯一索引",
        ),
        (
            options(ImportMode::Update, &["id", "name"], OnError::Stop),
            "至少还要给一列",
        ),
    ];
    for (options, expected) in cases {
        let error = import_csv(&sessions, &dir, csv, options).await.unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    assert_eq!(table(&sessions).await.len(), 3);
}

#[tokio::test]
async fn null_match_keys_are_failures_not_silent_misses() {
    let (dir, sessions) = open_sqlite(SCHEMA).await;
    let csv = "id,name\n,无名\n1,张三丰\n";
    // 遇错即停:检查阶段就拦下,什么都不写。
    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Update, &["id"], OnError::Stop),
    )
    .await
    .unwrap();
    assert_eq!((report.status, report.affected), (ImportStatus::Stopped, 0));
    assert_eq!(report.failures[0].column.as_deref(), Some("id"));
    assert!(report.failures[0].reason.contains("匹配键是空值"));
    assert_eq!(table(&sessions).await[0], "1 张三 老员工 A");

    let report = import_csv(
        &sessions,
        &dir,
        csv,
        options(ImportMode::Delete, &["id"], OnError::Skip),
    )
    .await
    .unwrap();
    assert_eq!(
        (report.affected, report.unmatched, report.failed),
        (1, 0, 1),
        "{}",
        report.summary
    );
}
