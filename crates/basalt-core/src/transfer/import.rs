//! 导入:读文件、按映射与目标列类型转换、一批一批参数化写进表。
//! 五种模式:追加、更新、追加或更新、删除、复制(先清空再追加),见 [`ImportMode`]。
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
use super::source::{SourceFormat, SourcePreview, SourceSpec, inspect};
use crate::design::TableDesign;
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

/// 文件里的行怎么写进表。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportMode {
    /// 追加:全部插入。
    #[default]
    Append,
    /// 更新:按匹配键找到表里的行,改掉其余映射了的列;找不到的行跳过,计入「未匹配」。
    Update,
    /// 追加或更新:按主键或唯一键,表里已有的更新,没有的插入。
    Upsert,
    /// 删除:按匹配键删掉表里对应的行;只用到匹配键这几列。
    Delete,
    /// 复制:先清空表(同一个事务里),再全部插入。前端要二次确认。
    Replace,
}

impl ImportMode {
    fn label(self) -> &'static str {
        match self {
            Self::Append => "追加",
            Self::Update => "更新",
            Self::Upsert => "追加或更新",
            Self::Delete => "删除",
            Self::Replace => "复制",
        }
    }

    fn uses_keys(self) -> bool {
        matches!(self, Self::Update | Self::Upsert | Self::Delete)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub on_error: OnError,
    pub batch_size: u32,
    #[serde(default)]
    pub mode: ImportMode,
    /// 匹配键(列名):更新、追加或更新、删除用它们找到表里对应的行。
    #[serde(default)]
    pub key_columns: Vec<String>,
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
    /// 导入结束时已经生效的表行数:插入、更新或删除了多少行。
    pub affected: u64,
    /// 更新、删除时在表里找不到对应行的文件行数。
    pub unmatched: u64,
    /// 失败的行数(含没带回明细的)。
    pub failed: u64,
    pub failures: Vec<ImportFailure>,
    /// 写给人看的结果说明,含 MySQL 哪些批已经生效。
    pub summary: String,
}

/// 表上的主键或一个唯一索引:「追加或更新」只能按它们匹配。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniqueKey {
    /// 索引名;主键为 `None`。
    pub name: Option<String>,
    pub columns: Vec<String>,
}

/// 导入向导第二步要的东西:文件预览、目标表的列、主键与唯一键和自动匹配的映射。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub source: SourcePreview,
    pub columns: Vec<ColumnInfo>,
    /// 主键在前。
    pub unique_keys: Vec<UniqueKey>,
    pub mapping: Vec<ColumnMapping>,
}

/// 主键与普通列上的唯一索引(表达式、部分索引不算:upsert 用不上)。
pub fn unique_keys(design: &TableDesign) -> Vec<UniqueKey> {
    let name_of = |id: &str| {
        design
            .columns
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
    };
    let primary = (!design.primary_key.is_empty()).then(|| UniqueKey {
        name: None,
        columns: design
            .primary_key
            .iter()
            .filter_map(|id| name_of(id))
            .collect(),
    });
    let indexes = design
        .indexes
        .iter()
        .filter(|index| index.unique && index.definition.is_none() && !index.columns.is_empty())
        .map(|index| UniqueKey {
            name: Some(index.name.clone()),
            columns: index
                .columns
                .iter()
                .filter_map(|c| name_of(&c.column))
                .collect(),
        });
    primary.into_iter().chain(indexes).collect()
}

/// 读不到表结构(权限、版本差异)时只按列信息里的主键兜底。
async fn table_keys(session: &Session, table: &TableRef, columns: &[ColumnInfo]) -> Vec<UniqueKey> {
    match session.read_design(table).await {
        Ok(design) => unique_keys(&design),
        Err(error) => {
            log::warn!("读不到 {} 的索引，只按主键匹配：{error}", table.table);
            let primary: Vec<String> = columns
                .iter()
                .filter(|c| c.is_primary_key)
                .map(|c| c.name.clone())
                .collect();
            if primary.is_empty() {
                Vec::new()
            } else {
                vec![UniqueKey {
                    name: None,
                    columns: primary,
                }]
            }
        }
    }
}

/// 匹配键在映射好的目标列里的位置。追加、复制不用键。
fn resolve_keys(
    mode: ImportMode,
    key_columns: &[String],
    targets: &[&ColumnInfo],
    unique: &[UniqueKey],
) -> CoreResult<Vec<usize>> {
    if !mode.uses_keys() {
        return Ok(Vec::new());
    }
    let label = mode.label();
    if key_columns.is_empty() {
        return Err(CoreError::InvalidData(format!(
            "「{label}」要选匹配键：按哪几列找到表里对应的行。"
        )));
    }
    let mut keys = Vec::with_capacity(key_columns.len());
    for name in key_columns {
        let index = targets
            .iter()
            .position(|c| &c.name == name)
            .ok_or_else(|| CoreError::InvalidData(format!("匹配键「{name}」没有选来源列。")))?;
        if !keys.contains(&index) {
            keys.push(index);
        }
    }
    if mode == ImportMode::Upsert {
        let chosen: Vec<&str> = keys.iter().map(|&i| targets[i].name.as_str()).collect();
        let is_unique = unique.iter().any(|key| {
            key.columns.len() == chosen.len()
                && key.columns.iter().all(|c| chosen.contains(&c.as_str()))
        });
        if !is_unique {
            return Err(CoreError::InvalidData(
                "「追加或更新」只能按主键或一个唯一索引匹配：否则数据库判断不了哪些行已经有了。"
                    .into(),
            ));
        }
    }
    if mode == ImportMode::Update && keys.len() == targets.len() {
        return Err(CoreError::InvalidData(
            "「更新」除了匹配键，至少还要给一列要改的列选上来源。".into(),
        ));
    }
    Ok(keys)
}

pub async fn preview_import(
    sessions: &SessionManager,
    connection_id: &str,
    table: &TableRef,
    spec: SourceSpec,
) -> CoreResult<ImportPreview> {
    let session = sessions.get(connection_id).await?;
    let columns = session.table_columns(table).await?;
    let unique_keys = table_keys(&session, table, &columns).await;
    let has_header = spec.has_header;
    let source = tokio::task::spawn_blocking(move || inspect(&spec))
        .await
        .map_err(|error| CoreError::File(error.to_string()))??;
    let mapping = auto_map(&source.headers, &columns, has_header);
    Ok(ImportPreview {
        source,
        columns,
        unique_keys,
        mapping,
    })
}

/// 导入用的 MySQL 连接打开严格模式:非严格模式下数据库会把装不下的字符换成 ?、把超范围的值截断、
/// 把不合法的时间存成零值,只给警告。这条连接导入完就关掉,不影响别的连接。
const STRICT_SESSION: &str = "SET SESSION sql_mode = CONCAT(@@SESSION.sql_mode, IF(@@SESSION.sql_mode = '', '', ','), 'STRICT_ALL_TABLES')";

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
    let csv = matches!(request.source.format, Some(SourceFormat::Csv { .. }));
    let mode = request.options.mode;
    if mode.uses_keys()
        && let Some(missing) = request.options.key_columns.iter().find(|key| {
            !request
                .mapping
                .iter()
                .any(|m| &m.column == *key && m.source.is_some())
        })
    {
        return Err(CoreError::InvalidData(format!(
            "匹配键「{missing}」没有选来源列。"
        )));
    }
    // 删除只用到匹配键:别的列不转换,也就不会因为它们的值不合法而失败。
    let mapping: Vec<ColumnMapping> = request
        .mapping
        .iter()
        .filter(|m| mode != ImportMode::Delete || request.options.key_columns.contains(&m.column))
        .cloned()
        .collect();
    let mut plan = Plan::new(
        dialect,
        columns.clone(),
        &mapping,
        request.options.binary,
        csv,
    )?;
    let unique = if mode == ImportMode::Upsert {
        table_keys(&session, &request.table, &columns).await
    } else {
        Vec::new()
    };
    let keys = {
        let targets: Vec<&ColumnInfo> = plan.targets.iter().map(|t| &t.column).collect();
        resolve_keys(mode, &request.options.key_columns, &targets, &unique)?
    };
    if matches!(mode, ImportMode::Update | ImportMode::Delete) {
        plan.non_null = keys.clone();
    }
    let targets: Vec<&ColumnInfo> = plan.targets.iter().map(|t| &t.column).collect();
    let transactional = transactional(&session, &request.table).await?;
    let stop_on_error = request.options.on_error == OnError::Stop;
    let mut failures = Failures::default();

    let total = if stop_on_error {
        let total = check(&plan, &request.source, &mut failures, cancel, &mut throttle).await?;
        if failures.count > 0 {
            return Ok(ImportReport {
                status: ImportStatus::Stopped,
                affected: 0,
                unmatched: 0,
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
    // 借不到(准备期间取消、连不上)也要把为这次导入新开的连接关掉。
    let mut lease = match sessions
        .lease_editor(&editor, &request.connection_id, &scope)
        .await
    {
        Ok(lease) => lease,
        Err(error) => {
            sessions.close_editor(&editor).await;
            return Err(error);
        }
    };
    if dialect == Dialect::Mysql
        && let Err(error) = lease.execute_sql(STRICT_SESSION).await
    {
        lease.release().await;
        sessions.close_editor(&editor).await;
        return Err(CoreError::Database(format!(
            "无法把导入用的连接设成严格模式：{error:?}"
        )));
    }
    let mut writer = BatchWriter {
        lease: &mut lease,
        dialect,
        table: &request.table,
        columns: targets,
        mode,
        keys,
        // 「复制」的清空要和插入一起生效或一起撤回:按批提交的话,清空随第一批提交,出错时原数据就没了。
        commit_each_batch: dialect == Dialect::Mysql && mode != ImportMode::Replace,
        transactional,
        stop_on_error,
        affected: 0,
        unmatched: 0,
        committed: 0,
        committed_unmatched: 0,
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
    let (affected, unmatched) = (writer.committed, writer.committed_unmatched);
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
        affected,
        unmatched,
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
    writer.begin().await?;
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

/// 顺利做完时按模式说做了什么。
fn done(writer: &BatchWriter<'_>) -> String {
    let n = writer.committed;
    let unmatched = match writer.committed_unmatched {
        0 => String::new(),
        m => format!("，文件里 {m} 行在表里没找到对应的行"),
    };
    match writer.mode {
        ImportMode::Append | ImportMode::Replace => format!("导入完成：成功 {n} 行"),
        ImportMode::Upsert => format!("导入完成：处理 {n} 行（表里已有的更新，没有的新增）"),
        ImportMode::Update => format!("更新完成：改了表里 {n} 行{unmatched}"),
        ImportMode::Delete => format!("删除完成：删掉表里 {n} 行{unmatched}"),
    }
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
                "这张表的存储引擎不支持事务：出错之前的改动（{} 行）已经生效{}。",
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
        Ok(()) => format!("{cleared}{}{skipped}。", done(writer)),
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

#[cfg(test)]
#[path = "import_mode_tests.rs"]
mod mode_tests;
