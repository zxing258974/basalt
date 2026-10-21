//! 导入模式在 MySQL / MariaDB / PostgreSQL 上:更新、追加或更新(按主键、按唯一索引)、删除。
//! 三家的 upsert 写法各不相同,SQLite 的在 basalt-core 的单测里。需要 BASALT_IT 选中目标(见 common)。

mod common;
mod transfer_support;

use basalt_core::session::SessionManager;
use basalt_core::table::TableRef;
use basalt_core::transfer::BinaryEncoding;
use basalt_core::transfer::import::{
    ImportMode, ImportOptions, ImportReport, ImportRequest, ImportStatus, OnError, preview_import,
    run_import,
};
use basalt_core::transfer::progress::{CancelFlag, ignore};
use basalt_core::transfer::source::SourceSpec;
use basalt_core::value::CellValue;
use common::*;
use transfer_support::{CONNECTION, ensure_postgres_database, rows};

const MYSQL_FIXTURE: &str = "
    CREATE DATABASE IF NOT EXISTS basalt_modes CHARACTER SET utf8mb4;
    DROP TABLE IF EXISTS basalt_modes.people;
    CREATE TABLE basalt_modes.people (
      id INT PRIMARY KEY,
      name VARCHAR(20) NOT NULL,
      code VARCHAR(10) UNIQUE,
      note TEXT
    ) ENGINE=InnoDB;
    INSERT INTO basalt_modes.people VALUES (1, '张三', 'A', '老员工'), (2, '李四', 'B', NULL), (3, '王五', 'C', '');
";

const PG_FIXTURE: &str = "
    DROP TABLE IF EXISTS public.modes_people;
    CREATE TABLE public.modes_people (
      id integer PRIMARY KEY,
      name varchar(20) NOT NULL,
      code varchar(10) UNIQUE,
      note text
    );
    INSERT INTO public.modes_people VALUES (1, '张三', 'A', '老员工'), (2, '李四', 'B', NULL), (3, '王五', 'C', '');
";

fn options(mode: ImportMode, keys: &[&str]) -> ImportOptions {
    ImportOptions {
        on_error: OnError::Stop,
        batch_size: 2,
        mode,
        key_columns: keys.iter().map(|k| (*k).to_owned()).collect(),
        binary: BinaryEncoding::Hex,
    }
}

async fn import_csv(
    sessions: &SessionManager,
    table: &TableRef,
    dir: &tempfile::TempDir,
    csv: &str,
    options: ImportOptions,
) -> ImportReport {
    let path = dir.path().join("in.csv");
    std::fs::write(&path, csv).unwrap();
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(sessions, CONNECTION, table, spec.clone())
        .await
        .unwrap();
    let request = ImportRequest {
        connection_id: CONNECTION.into(),
        table: table.clone(),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options,
    };
    let report = run_import(sessions, "modes", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        report.status,
        ImportStatus::Completed,
        "{}\n{:#?}",
        report.summary,
        report.failures
    );
    report
}

/// 每行拼成「id 名字 代码 备注」,NULL 写成 ∅。
async fn table_text(sessions: &SessionManager, table: &TableRef) -> Vec<String> {
    rows(sessions, table)
        .await
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

/// 四种模式依次做一遍,每步核对整张表。
async fn exercise(sessions: &SessionManager, table: &TableRef, label: &str) {
    let dir = tempfile::tempdir().unwrap();

    let report = import_csv(
        sessions,
        table,
        &dir,
        "id,name\n1,张三丰\n9,查无此人\n",
        options(ImportMode::Update, &["id"]),
    )
    .await;
    assert_eq!((report.affected, report.unmatched), (1, 1), "{label} 更新");

    let report = import_csv(
        sessions,
        table,
        &dir,
        "id,name,code\n2,李四四,B\n4,赵六,D\n5,钱七,E\n",
        options(ImportMode::Upsert, &["id"]),
    )
    .await;
    assert_eq!(report.affected, 3, "{label} 按主键追加或更新");

    // 按唯一索引;同一批里同一个代码出现两次(PostgreSQL 会拒绝整条语句,逐行重做),后面的说了算。
    let report = import_csv(
        sessions,
        table,
        &dir,
        "code,id,name\nA,1,张三（改）\nF,6,孙八\nF,6,孙八八\n",
        options(ImportMode::Upsert, &["code"]),
    )
    .await;
    assert_eq!(report.failed, 0, "{label} 按唯一索引追加或更新");

    let report = import_csv(
        sessions,
        table,
        &dir,
        "id\n3\n8\n",
        options(ImportMode::Delete, &["id"]),
    )
    .await;
    assert_eq!((report.affected, report.unmatched), (1, 1), "{label} 删除");

    assert_eq!(
        table_text(sessions, table).await,
        [
            "1 张三（改） A 老员工",
            "2 李四四 B ∅",
            "4 赵六 D ∅",
            "5 钱七 E ∅",
            "6 孙八八 F ∅",
        ],
        "{label}"
    );
}

/// 「复制」在 MySQL 上也是一个事务:后面的批被拒绝时,清空和前面的批一起撤回,原来的行都还在。
async fn replace_is_all_or_nothing(sessions: &SessionManager, table: &TableRef, label: &str) {
    let dir = tempfile::tempdir().unwrap();
    let before = table_text(sessions, table).await;
    let path = dir.path().join("replace.csv");
    // 每批 2 行:第 3 批里主键重复。
    std::fs::write(
        &path,
        "id,name\n10,甲\n11,乙\n12,丙\n13,丁\n14,戊\n10,重复\n",
    )
    .unwrap();
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(sessions, CONNECTION, table, spec.clone())
        .await
        .unwrap();
    let request = ImportRequest {
        connection_id: CONNECTION.into(),
        table: table.clone(),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options: options(ImportMode::Replace, &[]),
    };
    let report = run_import(
        sessions,
        "replace",
        &request,
        &CancelFlag::default(),
        &ignore,
    )
    .await
    .unwrap();
    assert_eq!(
        (report.status, report.affected),
        (ImportStatus::Stopped, 0),
        "{label}：{}",
        report.summary
    );
    assert!(
        report.summary.contains("整个导入已经回滚"),
        "{label}：{}",
        report.summary
    );
    assert_eq!(
        table_text(sessions, table).await,
        before,
        "{label}：原来的行都还在"
    );
}

/// 「复制」导到一半取消:清空和已写的批一起撤回。3000 行、每批 100 行,第一批写完就取消。
async fn replace_cancelled_midway_rolls_back(
    sessions: &SessionManager,
    table: &TableRef,
    label: &str,
) {
    let dir = tempfile::tempdir().unwrap();
    let before = table_text(sessions, table).await;
    let path = dir.path().join("many.csv");
    let body: String = (100..3100).map(|id| format!("{id},名{id}\n")).collect();
    std::fs::write(&path, format!("id,name\n{body}")).unwrap();
    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(sessions, CONNECTION, table, spec.clone())
        .await
        .unwrap();
    let request = ImportRequest {
        connection_id: CONNECTION.into(),
        table: table.clone(),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options: ImportOptions {
            on_error: OnError::Skip,
            batch_size: 100,
            ..options(ImportMode::Replace, &[])
        },
    };
    let cancel = CancelFlag::default();
    let stop = cancel.clone();
    // 进度有节流;阶段一变就会报一次:第一批写完、报「导入中」时就取消。
    let progress = move |p: &basalt_core::transfer::progress::Progress| {
        if p.phase == basalt_core::transfer::progress::Phase::Importing {
            stop.cancel();
        }
    };
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_import(sessions, "replace-cancel", &request, &cancel, &progress),
    )
    .await
    .unwrap_or_else(|_| panic!("{label}：取消后 60 秒还没结束"))
    .unwrap();
    assert_eq!(
        (report.status, report.affected),
        (ImportStatus::Cancelled, 0),
        "{label}：{}",
        report.summary
    );
    assert_eq!(
        table_text(sessions, table).await,
        before,
        "{label}：原来的行都还在"
    );
}

#[tokio::test]
async fn mysql_family_supports_every_import_mode() {
    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let pool = sqlx::MySqlPool::connect(&mysql_url(target))
            .await
            .expect("连接 MySQL 测试容器");
        sqlx::raw_sql(MYSQL_FIXTURE).execute(&pool).await.unwrap();
        pool.close().await;
        let sessions = SessionManager::default();
        sessions
            .open(CONNECTION, &params(target, TEST_PASSWORD, None))
            .await
            .unwrap();
        let table = TableRef {
            database: "basalt_modes".into(),
            schema: None,
            table: "people".into(),
        };
        replace_is_all_or_nothing(&sessions, &table, target.name).await;
        replace_cancelled_midway_rolls_back(&sessions, &table, target.name).await;
        exercise(&sessions, &table, target.name).await;
    }
}

#[tokio::test]
async fn postgres_supports_every_import_mode() {
    if !selected(POSTGRES) {
        return;
    }
    ensure_postgres_database(&postgres_url("postgres"), FIXTURE_DB).await;
    let pool = sqlx::PgPool::connect(&postgres_url(FIXTURE_DB))
        .await
        .unwrap();
    sqlx::raw_sql(PG_FIXTURE).execute(&pool).await.unwrap();
    pool.close().await;
    let sessions = SessionManager::default();
    sessions
        .open(
            CONNECTION,
            &params(POSTGRES, TEST_PASSWORD, Some(FIXTURE_DB)),
        )
        .await
        .unwrap();
    let table = TableRef {
        database: FIXTURE_DB.into(),
        schema: Some("public".into()),
        table: "modes_people".into(),
    };
    replace_is_all_or_nothing(&sessions, &table, "PostgreSQL").await;
    replace_cancelled_midway_rolls_back(&sessions, &table, "PostgreSQL").await;
    exercise(&sessions, &table, "PostgreSQL").await;
}
