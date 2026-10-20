//! 10 万行的导出导入:内存不随行数涨、能中途取消(走查询标签页同一套取消)、导回来行数一致。
//! 每行带 1 KB 的文本,10 万行约 100 MB:要是把行攒在内存里,进程内存会涨上去。
//! 设了 BASALT_IT 才跑(SQLite 总在其中,MySQL / MariaDB / PostgreSQL 看是否选中)。所有目标在一个测试里依次跑,
//! 量内存时不被同一进程里的别的测试干扰。

mod common;
mod transfer_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use basalt_core::session::SessionManager;
use basalt_core::table::TableRef;
use basalt_core::transfer::export::{ExportRequest, ExportSource, run_export, transfer_editor_id};
use basalt_core::transfer::import::{
    ImportOptions, ImportRequest, ImportStatus, OnError, preview_import, run_import,
};
use basalt_core::transfer::progress::{CancelFlag, Progress, ignore};
use basalt_core::transfer::source::SourceSpec;
use basalt_core::transfer::{BinaryEncoding, CsvOptions, ExportFormat, ExportOptions};
use common::*;
use transfer_support::*;

const ROWS: u64 = 100_000;
/// 导出期间进程内存最多比开始时多这么多。数据有 100 MB,攒在内存里一定超过。
const MAX_GROWTH_KB: u64 = 48 * 1024;

/// 后台每 50 毫秒量一次本进程的常驻内存(KB),记下最大值。
struct RssProbe {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn rss_kb() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

impl RssProbe {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(rss_kb()));
        let (s, p) = (stop.clone(), peak.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss_kb(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Self {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        self.peak.load(Ordering::Relaxed).max(rss_kb())
    }
}

fn request(table: &TableRef, format: ExportFormat, path: &std::path::Path) -> ExportRequest {
    ExportRequest {
        connection_id: CONNECTION.into(),
        source: ExportSource::Table {
            table: table.clone(),
            filter: None,
            sort: None,
        },
        options: ExportOptions {
            format,
            binary: BinaryEncoding::Hex,
        },
        path: path.to_string_lossy().into_owned(),
    }
}

async fn exports_without_growing_memory(sessions: &SessionManager, table: &TableRef, label: &str) {
    let dir = tempfile::tempdir().unwrap();
    for (extension, format) in [
        ("csv", ExportFormat::Csv(CsvOptions::default())),
        ("xlsx", ExportFormat::Xlsx),
    ] {
        let path = dir.path().join(format!("big.{extension}"));
        let before = rss_kb();
        let probe = RssProbe::start();
        let started = Instant::now();
        let outcome = run_export(
            sessions,
            "big",
            &request(table, format, &path),
            &CancelFlag::default(),
            &ignore,
        )
        .await
        .unwrap();
        let peak = probe.finish();
        let growth = peak.saturating_sub(before);
        println!(
            "{label} {extension}：{} 行，文件 {} MB，用时 {:.1} 秒，内存开始 {} MB、峰值 {} MB（多 {} MB）",
            outcome.rows,
            outcome.bytes / 1_048_576,
            started.elapsed().as_secs_f64(),
            before / 1024,
            peak / 1024,
            growth / 1024
        );
        assert_eq!(outcome.rows, ROWS, "{label} {extension}");
        assert!(
            growth < MAX_GROWTH_KB,
            "{label} {extension} 导出时内存涨了 {} MB",
            growth / 1024
        );
    }
}

/// 读到 5000 行以后从另一个任务取消(查询标签页同一个入口),应该很快停下、不留文件,连接还能用。
async fn export_can_be_cancelled_midway(
    sessions: &Arc<SessionManager>,
    table: &TableRef,
    label: &str,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cancel.csv");
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let progress = move |p: &Progress| {
        let _ = sender.send(p.rows);
    };
    let job = {
        let (sessions, request) = (
            sessions.clone(),
            request(table, ExportFormat::Csv(CsvOptions::default()), &path),
        );
        tokio::spawn(async move {
            run_export(
                &sessions,
                "cancel-job",
                &request,
                &CancelFlag::default(),
                &progress,
            )
            .await
        })
    };
    while let Some(rows) = receiver.recv().await {
        if rows >= 5000 {
            break;
        }
    }
    let started = Instant::now();
    if !sessions
        .cancel_query(&transfer_editor_id("cancel-job"))
        .await
        .unwrap()
    {
        panic!(
            "{label}：取消时导出应该正在执行，实际结果：{:?}",
            job.await.unwrap()
        );
    }
    let error = job.await.unwrap().unwrap_err();
    println!(
        "{label} 取消导出：{:.2} 秒停下",
        started.elapsed().as_secs_f64()
    );
    assert_eq!(error.code(), "cancelled", "{label}：{error}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{label}：取消太慢"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "{label}：不该留下文件"
    );
    // 连接还好好的。
    let rows = rows_where(sessions, table, "id <= 3").await;
    assert_eq!(rows, 3, "{label}");
}

async fn rows_where(sessions: &SessionManager, table: &TableRef, filter: &str) -> u64 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("n.csv");
    let mut request = request(table, ExportFormat::Json { lines: true }, &path);
    if let ExportSource::Table { filter: slot, .. } = &mut request.source {
        *slot = Some(filter.to_owned());
    }
    run_export(sessions, "count", &request, &CancelFlag::default(), &ignore)
        .await
        .unwrap()
        .rows
}

/// 10 万行 CSV 导回空表;再导一次并在中途取消:PostgreSQL、SQLite 整体回滚,MySQL 报告已提交的批次。
async fn imports_and_cancels(
    sessions: &SessionManager,
    table: &TableRef,
    copy: &TableRef,
    label: &str,
    mysql: bool,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.csv");
    export_table(
        sessions,
        table,
        ExportFormat::Csv(CsvOptions::default()),
        &path,
    )
    .await;
    let started = Instant::now();
    let report = import_file_in_batches(sessions, copy, &path, 1000).await;
    println!(
        "{label} 导入 {} 行：{:.1} 秒",
        report.inserted,
        started.elapsed().as_secs_f64()
    );
    assert_eq!(
        (report.status, report.inserted, report.failed),
        (ImportStatus::Completed, ROWS, 0),
        "{label}：{}",
        report.summary
    );

    let spec = SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header: true,
    };
    let preview = preview_import(sessions, CONNECTION, copy, spec.clone())
        .await
        .unwrap();
    let request = ImportRequest {
        connection_id: CONNECTION.into(),
        table: copy.clone(),
        source: SourceSpec {
            format: Some(preview.source.format),
            ..spec
        },
        mapping: preview.mapping,
        options: ImportOptions {
            on_error: OnError::Skip,
            batch_size: 1000,
            clear_table: true,
            binary: BinaryEncoding::Hex,
        },
    };
    let cancel = CancelFlag::default();
    let stop = cancel.clone();
    let progress = move |p: &Progress| {
        if p.rows >= 20_000 {
            stop.cancel();
        }
    };
    let report = run_import(sessions, "import-cancel", &request, &cancel, &progress)
        .await
        .unwrap();
    println!("{label} 取消导入：{}", report.summary);
    assert_eq!(report.status, ImportStatus::Cancelled, "{label}");
    let left = rows_where(sessions, copy, "1 = 1").await;
    if mysql {
        assert!(
            report.inserted >= 20_000 && report.inserted < ROWS,
            "{label}：{}",
            report.summary
        );
        assert_eq!(
            left, report.inserted,
            "{label}：报告里说的已生效行数要和表里一致"
        );
        assert!(
            report.summary.contains("已经生效"),
            "{label}：{}",
            report.summary
        );
    } else {
        assert_eq!(
            (report.inserted, left),
            (0, ROWS),
            "{label}：整体回滚，清空也撤回"
        );
    }
}

// 多线程运行时:和应用里一样,导出在别的线程上跑,取消请求能插进来。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hundred_thousand_rows() {
    // 很慢(调试构建下几分钟):只在跑集成测试(设了 BASALT_IT)时跑,SQLite 也一样。
    if std::env::var("BASALT_IT").is_err() {
        return;
    }
    // SQLite
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(
        dir.path(),
        "big.db",
        "CREATE TABLE big (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, amount DECIMAL(12,2), created DATETIME);
         CREATE TABLE big_copy (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, amount DECIMAL(12,2), created DATETIME);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000)
         INSERT INTO big SELECT i, '名字' || i, printf('%.1000c', 'x'), i * 1.25, '2026-10-20 08:00:00' FROM n;",
    )
    .await;
    let sessions = Arc::new(SessionManager::default());
    sessions
        .open(CONNECTION, &sqlite_params(&path))
        .await
        .unwrap();
    let table = |name: &str| TableRef {
        database: "main".into(),
        schema: None,
        table: name.into(),
    };
    exports_without_growing_memory(&sessions, &table("big"), "sqlite").await;
    export_can_be_cancelled_midway(&sessions, &table("big"), "sqlite").await;
    imports_and_cancels(
        &sessions,
        &table("big"),
        &table("big_copy"),
        "sqlite",
        false,
    )
    .await;
    sessions.close(CONNECTION).await;

    for target in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let pool = sqlx::MySqlPool::connect(&mysql_url(target)).await.unwrap();
        sqlx::raw_sql(
            "CREATE DATABASE IF NOT EXISTS basalt_m6 CHARACTER SET utf8mb4;
             DROP TABLE IF EXISTS basalt_m6.big, basalt_m6.big_copy, basalt_m6.digits;
             CREATE TABLE basalt_m6.digits (n INT PRIMARY KEY);
             INSERT INTO basalt_m6.digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);
             CREATE TABLE basalt_m6.big (id INT PRIMARY KEY, name VARCHAR(40) NOT NULL, note TEXT,
                                         amount DECIMAL(12,2), created DATETIME);
             CREATE TABLE basalt_m6.big_copy LIKE basalt_m6.big;
             INSERT INTO basalt_m6.big
             SELECT a.n + b.n * 10 + c.n * 100 + d.n * 1000 + e.n * 10000 + 1 AS id,
                    CONCAT('名字', a.n + b.n * 10 + c.n * 100 + d.n * 1000 + e.n * 10000 + 1),
                    REPEAT('x', 1000), (a.n + b.n * 10) * 1.25, '2026-10-20 08:00:00'
             FROM basalt_m6.digits a, basalt_m6.digits b, basalt_m6.digits c, basalt_m6.digits d, basalt_m6.digits e;",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let sessions = Arc::new(SessionManager::default());
        sessions
            .open(CONNECTION, &params(target, TEST_PASSWORD, None))
            .await
            .unwrap();
        let table = |name: &str| TableRef {
            database: "basalt_m6".into(),
            schema: None,
            table: name.into(),
        };
        exports_without_growing_memory(&sessions, &table("big"), target.name).await;
        export_can_be_cancelled_midway(&sessions, &table("big"), target.name).await;
        imports_and_cancels(
            &sessions,
            &table("big"),
            &table("big_copy"),
            target.name,
            true,
        )
        .await;
        sessions.close(CONNECTION).await;
    }

    if selected(POSTGRES) {
        ensure_postgres_database(&postgres_url("postgres"), FIXTURE_DB).await;
        let pool = sqlx::PgPool::connect(&postgres_url(FIXTURE_DB))
            .await
            .unwrap();
        sqlx::raw_sql(
            "DROP TABLE IF EXISTS public.m6_big, public.m6_big_copy;
             CREATE TABLE public.m6_big (id integer PRIMARY KEY, name varchar(40) NOT NULL, note text,
                                         amount numeric(12,2), created timestamp);
             CREATE TABLE public.m6_big_copy (LIKE public.m6_big INCLUDING ALL);
             INSERT INTO public.m6_big
             SELECT i, '名字' || i, repeat('x', 1000), i * 1.25, '2026-10-20 08:00:00' FROM generate_series(1, 100000) AS i;",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let sessions = Arc::new(SessionManager::default());
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
        exports_without_growing_memory(&sessions, &table("m6_big"), "postgres").await;
        export_can_be_cancelled_midway(&sessions, &table("m6_big"), "postgres").await;
        imports_and_cancels(
            &sessions,
            &table("m6_big"),
            &table("m6_big_copy"),
            "postgres",
            false,
        )
        .await;
        sessions.close(CONNECTION).await;
    }
}
