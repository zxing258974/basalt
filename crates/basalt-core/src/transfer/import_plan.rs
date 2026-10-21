//! 导入的计划与读文件:映射好的列按目标类型转换一行,文件在单独的线程里一块一块读出来。

use tokio::sync::mpsc;

use super::BinaryEncoding;
use super::convert::convert;
use super::mapping::ColumnMapping;
use super::progress::{CancelFlag, Phase, Progress, Throttle};
use super::report::{Failures, ImportFailure};
use super::source::{SourceRecord, SourceSpec, SourceValue, read_rows};
use super::target::{TargetKind, classify};
use crate::error::{CoreError, CoreResult};
use crate::sql::Dialect;
use crate::sql::bind::BindValue;
use crate::table::ColumnInfo;

/// 读文件的线程一次交出多少行。
const CHUNK_ROWS: usize = 500;

/// 映射好的一列:写哪一列、按什么类型转、取来源第几列。
pub(super) struct Target {
    pub column: ColumnInfo,
    kind: TargetKind,
    source: usize,
}

pub(super) struct Plan {
    dialect: Dialect,
    pub targets: Vec<Target>,
    binary: BinaryEncoding,
    /// CSV 的每一行字段数都应该和列名行一样;Excel 的行末尾空着的格子本来就不算。
    csv: bool,
}

impl Plan {
    pub(super) fn new(
        dialect: Dialect,
        columns: Vec<ColumnInfo>,
        mapping: &[ColumnMapping],
        binary: BinaryEncoding,
        csv: bool,
    ) -> CoreResult<Self> {
        let mut targets = Vec::new();
        for item in mapping {
            let Some(source) = item.source else { continue };
            let column = columns
                .iter()
                .find(|c| c.name == item.column)
                .ok_or_else(|| {
                    CoreError::InvalidData(format!(
                        "表里没有列「{}」，请重新打开导入向导。",
                        item.column
                    ))
                })?;
            if column.is_generated {
                return Err(CoreError::InvalidData(format!(
                    "「{}」是生成列，不能导入。",
                    column.name
                )));
            }
            targets.push(Target {
                kind: classify(dialect, column),
                column: column.clone(),
                source,
            });
        }
        if targets.is_empty() {
            return Err(CoreError::InvalidData("至少要给一列选上来源。".into()));
        }
        Ok(Self {
            dialect,
            targets,
            binary,
            csv,
        })
    }

    /// 一行转成参数;任何一格转换不了,整行失败。`width` 是列名行的列数:
    /// 一行比它多出非空的字段,多半是值里有没加引号的分隔符;CSV 的一行比它少,多半是值里有没加引号的
    /// 换行。数据已经错位,整行算失败,不拿 NULL 补齐。
    pub(super) fn convert(
        &self,
        record: &SourceRecord,
        width: Option<usize>,
    ) -> Result<Vec<BindValue>, ImportFailure> {
        let used = record
            .values
            .iter()
            .rposition(|v| *v != SourceValue::Null)
            .map_or(0, |i| i + 1);
        if let Some(width) = width
            && used > width
        {
            return Err(ImportFailure {
                row: record.row,
                column: None,
                value: None,
                reason: format!(
                    "这一行有 {used} 个字段，比列名行多 {} 个，可能是值里有没加引号的分隔符",
                    used - width
                ),
            });
        }
        if let Some(width) = width
            && self.csv
            && record.values.len() < width
        {
            return Err(ImportFailure {
                row: record.row,
                column: None,
                value: None,
                reason: format!(
                    "这一行只有 {} 个字段，比列名行少 {} 个，可能是值里有没加引号的换行",
                    record.values.len(),
                    width - record.values.len()
                ),
            });
        }
        self.targets
            .iter()
            .map(|target| {
                let value = record
                    .values
                    .get(target.source)
                    .unwrap_or(&SourceValue::Null);
                convert(
                    self.dialect,
                    &target.column,
                    &target.kind,
                    value,
                    self.binary,
                )
                .map_err(|reason| ImportFailure {
                    row: record.row,
                    column: Some(target.column.name.clone()),
                    value: value.display(),
                    reason,
                })
            })
            .collect()
    }
}

pub(super) enum ReadEvent {
    /// 列名行有几列。
    Width(usize),
    Rows(Vec<SourceRecord>),
    Failed(CoreError),
}

/// 在单独的线程里读文件,一块一块交出来。接收端不要了(取消、出错)线程就停下。
pub(super) fn spawn_reader(spec: SourceSpec) -> mpsc::Receiver<ReadEvent> {
    let (sender, receiver) = mpsc::channel(4);
    std::thread::spawn(move || {
        let mut chunk = Vec::with_capacity(CHUNK_ROWS);
        let headers = |names: Vec<String>| {
            // 接收端已经不要了也没关系,下面送数据时会发现。
            let _ = sender.blocking_send(ReadEvent::Width(names.len()));
        };
        let result = read_rows(&spec, headers, |record| {
            chunk.push(record);
            if chunk.len() < CHUNK_ROWS {
                return Ok(true);
            }
            Ok(sender
                .blocking_send(ReadEvent::Rows(std::mem::take(&mut chunk)))
                .is_ok())
        });
        let last = match result {
            Ok(()) if chunk.is_empty() => return,
            Ok(()) => ReadEvent::Rows(chunk),
            Err(error) => ReadEvent::Failed(error),
        };
        // 接收端已经不要了也没关系。
        let _ = sender.blocking_send(last);
    });
    receiver
}

/// 遇错即停时先过一遍整份文件,只转换不写。返回数据行数。
pub(super) async fn check(
    plan: &Plan,
    spec: &SourceSpec,
    failures: &mut Failures,
    cancel: &CancelFlag,
    throttle: &mut Throttle<'_>,
) -> CoreResult<u64> {
    let mut reader = spawn_reader(spec.clone());
    let mut rows = 0;
    let mut width = None;
    while let Some(event) = reader.recv().await {
        let records = match event {
            ReadEvent::Width(columns) => {
                width = Some(columns);
                continue;
            }
            ReadEvent::Rows(records) => records,
            ReadEvent::Failed(error) => return Err(error),
        };
        for record in &records {
            if let Err(failure) = plan.convert(record, width) {
                failures.push(failure);
            }
        }
        rows += records.len() as u64;
        if cancel.is_cancelled() {
            return Err(CoreError::Cancelled);
        }
        throttle.report(&Progress {
            failed: failures.count,
            ..Progress::new(Phase::Checking, rows)
        });
    }
    Ok(rows)
}
