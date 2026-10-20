//! 导入:读文件、按映射与目标列类型转换、一批一批参数化写进表。
//!
//! 遇错即停时先把整份文件检查一遍,有转换不了的值就什么都不写;写入时数据库拒绝的行
//! (主键冲突、外键……)逐行找出来记进报告。文件在单独的线程里读,读一块交一块,不整份读进内存。

use serde::{Deserialize, Serialize};

use super::BinaryEncoding;
use super::import_batch::{Abort, BatchWriter, Row};
use super::import_plan::{Plan, ReadEvent, check, spawn_reader};
use super::mapping::{ColumnMapping, auto_map};
use super::progress::{CancelFlag, Phase, Progress, ProgressSink, Throttle};
use super::report::{Failures, ImportFailure};
use super::source::{SourcePreview, SourceSpec, inspect};
use crate::error::{CoreError, CoreResult};
use crate::query::QueryScope;
use crate::session::{Session, SessionManager};
use crate::sql::Dialect;
use crate::table::{ColumnInfo, TableRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnError {
    /// 什么都不写(PostgreSQL、SQLite 整体回滚;MySQL 撤回当前这批)。
    Stop,
    /// 跳过出错的行,记进报告,其余照常导入。
    Skip,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub on_error: OnError,
    pub batch_size: u32,
    /// 导入前清空表(前端要二次确认)。
    pub clear_table: bool,
    /// 二进制列里的文本是怎么写的。
    #[serde(default)]
    pub binary: BinaryEncoding,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub connection_id: String,
    pub table: TableRef,
    /// 格式要已经定下来(预览时探测好的)。
    pub source: SourceSpec,
    pub mapping: Vec<ColumnMapping>,
    pub options: ImportOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStatus {
    Completed,
    /// 遇错即停,停下了。
    Stopped,
    Cancelled,
    /// 连接断了、文件读坏了、提交失败……
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub status: ImportStatus,
    /// 导入结束时已经在表里生效的行数。
    pub inserted: u64,
    /// 失败的行数(含没带回明细的)。
    pub failed: u64,
    pub failures: Vec<ImportFailure>,
    /// 写给人看的结果说明,含 MySQL 哪些批已经生效。
    pub summary: String,
}

/// 导入向导第二步要的东西:文件预览、目标表的列和自动匹配的映射。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub source: SourcePreview,
    pub columns: Vec<ColumnInfo>,
    pub mapping: Vec<ColumnMapping>,
}

pub async fn preview_import(
    sessions: &SessionManager,
    connection_id: &str,
    table: &TableRef,
    spec: SourceSpec,
) -> CoreResult<ImportPreview> {
    let session = sessions.get(connection_id).await?;
    let columns = session.table_columns(table).await?;
    let has_header = spec.has_header;
    let source = tokio::task::spawn_blocking(move || inspect(&spec))
        .await
        .map_err(|error| CoreError::File(error.to_string()))??;
    let mapping = auto_map(&source.headers, &columns, has_header);
    Ok(ImportPreview {
        source,
        columns,
        mapping,
    })
}

/// MySQL 的表用的是不是支持事务的引擎。
async fn transactional(session: &Session, table: &TableRef) -> CoreResult<bool> {
    let Session::Mysql(mysql) = session else {
        return Ok(true);
    };
    let engine = mysql.table_engine(table).await?.unwrap_or_default();
    Ok(["InnoDB", "NDB", "NDBCLUSTER", "RocksDB", "TokuDB"]
        .iter()
        .any(|known| known.eq_ignore_ascii_case(&engine)))
}

/// 执行一次导入。数据库层面的失败写在报告里,不作为 `Err` 返回;连接没打开、
/// 表不存在这类开始之前的问题才是 `Err`。
pub async fn run_import(
    sessions: &SessionManager,
    job_id: &str,
    request: &ImportRequest,
    cancel: &CancelFlag,
    progress: &dyn ProgressSink,
) -> CoreResult<ImportReport> {
    let mut throttle = Throttle::new(progress);
    throttle.force(&Progress::new(Phase::Preparing, 0));
    let session = sessions.get(&request.connection_id).await?;
    let dialect = session.dialect();
    let columns = session.table_columns(&request.table).await?;
    let plan = Plan::new(dialect, columns, &request.mapping, request.options.binary)?;
    let transactional = transactional(&session, &request.table).await?;
    let stop_on_error = request.options.on_error == OnError::Stop;
    let mut failures = Failures::default();

    let total = if stop_on_error {
        let total = check(&plan, &request.source, &mut failures, cancel, &mut throttle).await?;
        if failures.count > 0 {
            return Ok(ImportReport {
                status: ImportStatus::Stopped,
                inserted: 0,
                failed: failures.count,
                summary: format!(
                    "检查时发现 {} 行的值不能按目标列解析，什么都没有写入。可以改好文件再导入，或者选「跳过出错的行」。",
                    failures.count
                ),
                failures: failures.items,
            });
        }
        Some(total)
    } else {
        None
    };

    let editor = super::export::transfer_editor_id(job_id);
    let scope = QueryScope {
        database: Some(request.table.database.clone()),
        schema: None,
    };
    let mut lease = sessions
        .lease_editor(&editor, &request.connection_id, &scope)
        .await?;
    let mut writer = BatchWriter {
        lease: &mut lease,
        dialect,
        table: &request.table,
        columns: plan.targets.iter().map(|t| &t.column).collect(),
        commit_each_batch: dialect == Dialect::Mysql,
        transactional,
        stop_on_error,
        inserted: 0,
        committed: 0,
        committed_batches: 0,
        committed_through: None,
        cleared: None,
    };
    let outcome = write_all(
        &plan,
        request,
        &mut writer,
        &mut failures,
        cancel,
        &mut throttle,
        total,
    )
    .await;
    if outcome.is_err() {
        writer.abandon().await;
    }
    let summary = summarize(&writer, &outcome, &failures, dialect);
    let inserted = writer.committed;
    drop(writer);
    lease.release().await;
    sessions.close_editor(&editor).await;
    let status = match &outcome {
        Ok(()) => ImportStatus::Completed,
        Err(Abort::Stopped) => ImportStatus::Stopped,
        Err(Abort::Cancelled) => ImportStatus::Cancelled,
        Err(_) => ImportStatus::Failed,
    };
    Ok(ImportReport {
        status,
        inserted,
        failed: failures.count,
        failures: failures.items,
        summary,
    })
}

async fn write_all(
    plan: &Plan,
    request: &ImportRequest,
    writer: &mut BatchWriter<'_>,
    failures: &mut Failures,
    cancel: &CancelFlag,
    throttle: &mut Throttle<'_>,
    total: Option<u64>,
) -> Result<(), Abort> {
    writer.begin(request.options.clear_table).await?;
    let batch_size = request.options.batch_size.clamp(1, 10_000) as usize;
    let mut reader = spawn_reader(request.source.clone());
    let mut batch: Vec<Row> = Vec::with_capacity(batch_size);
    let mut seen = 0u64;
    let mut width = None;
    while let Some(event) = reader.recv().await {
        let records = match event {
            ReadEvent::Width(columns) => {
                width = Some(columns);
                continue;
            }
            ReadEvent::Rows(records) => records,
            ReadEvent::Failed(error) => return Err(Abort::Lost(error.to_string())),
        };
        for record in records {
            seen += 1;
            match plan.convert(&record, width) {
                Ok(values) => batch.push((record.row, values)),
                Err(failure) => {
                    failures.push(failure);
                    if writer.stop_on_error {
                        return Err(Abort::Stopped);
                    }
                }
            }
            if batch.len() >= batch_size {
                if cancel.is_cancelled() || writer.lease.cancel_requested() {
                    return Err(Abort::Cancelled);
                }
                writer.write(std::mem::take(&mut batch), failures).await?;
                throttle.report(&Progress {
                    failed: failures.count,
                    fraction: total.map(|t| seen as f64 / t.max(1) as f64),
                    ..Progress::new(Phase::Importing, seen)
                });
            }
        }
    }
    if cancel.is_cancelled() {
        return Err(Abort::Cancelled);
    }
    writer.write(batch, failures).await?;
    throttle.force(&Progress {
        failed: failures.count,
        fraction: Some(1.0),
        ..Progress::new(Phase::Finishing, seen)
    });
    writer.finish().await
}

/// 结果说明。MySQL 每批提交,中途停下时要说清楚哪些已经生效。
fn summarize(
    writer: &BatchWriter<'_>,
    outcome: &Result<(), Abort>,
    failures: &Failures,
    dialect: Dialect,
) -> String {
    let cleared = match writer.cleared {
        Some(rows) => format!("导入前清空了表（删掉 {rows} 行）。"),
        None => String::new(),
    };
    let skipped = if failures.count > 0 {
        format!("，{} 行失败（见下面的明细）", failures.count)
    } else {
        String::new()
    };
    let kept = || {
        if !writer.transactional {
            return format!(
                "这张表的存储引擎不支持事务：出错之前写进去的 {} 行已经生效{}。",
                writer.committed,
                if writer.cleared.is_some() {
                    "，清空表也已生效"
                } else {
                    ""
                }
            );
        }
        match (dialect, writer.committed_batches, writer.committed_through) {
            (Dialect::Mysql, batches, Some(row)) if batches > 0 => format!(
                "MySQL 每批单独提交：前 {batches} 批（到文件第 {row} 行，共 {} 行）已经生效{}，正在写的这一批已撤回。",
                writer.committed,
                if writer.cleared.is_some() {
                    "，导入前的清空也已生效"
                } else {
                    ""
                }
            ),
            _ => "整个导入已经回滚，表没有变化。".into(),
        }
    };
    match outcome {
        Ok(()) => format!("{cleared}导入完成：成功 {} 行{skipped}。", writer.committed),
        Err(Abort::Stopped) => {
            let row = failures.items.last().map_or(0, |f| f.row);
            format!("文件第 {row} 行被数据库拒绝，已停止。{}", kept())
        }
        Err(Abort::Cancelled) => format!("已取消。{}", kept()),
        Err(Abort::Lost(message)) => format!("导入中途出错：{message}。{}", kept()),
        Err(Abort::CommitFailed(message)) => {
            format!("提交（COMMIT）时出错，无法确定最后这一批是否已经生效，请刷新后核对：{message}")
        }
    }
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
