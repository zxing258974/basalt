//! 导出:从表或查询流式读出、交给写器。数据在专用连接上一行一行读,写器边收边写,
//! 内存不随行数增长;取消、出错都不留下半个文件。

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::progress::{CancelFlag, Phase, Progress, ProgressSink, Throttle};
use super::writer::{OutputFile, RowWriter, open_writer};
use super::{ExportColumn, ExportFormat, ExportOptions};
use crate::error::{CoreError, CoreResult};
use crate::query::lease::{ExecError, RowSink, StreamEnd, StreamQuery};
use crate::query::{QueryScope, ResultColumn, leading_keyword};
use crate::session::SessionManager;
use crate::sql::Dialect;
use crate::sql::select::select_all;
use crate::table::{SortSpec, TableRef};
use crate::value::CellValue;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportSource {
    /// 整张表,或表数据页当前筛选条件下的全部结果。
    #[serde(rename_all = "camelCase")]
    Table {
        table: TableRef,
        #[serde(default)]
        filter: Option<String>,
        #[serde(default)]
        sort: Option<SortSpec>,
    },
    /// 查询结果超出显示上限:在查询标签页自己的连接上重新执行这条语句,读出全部结果。
    #[serde(rename_all = "camelCase")]
    Query {
        editor_id: String,
        scope: QueryScope,
        sql: String,
    },
    /// 查询结果已经完整显示了:直接写这些行,不再执行一遍。
    #[serde(rename_all = "camelCase")]
    Rows {
        columns: Vec<ResultColumn>,
        rows: Vec<Vec<CellValue>>,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub connection_id: String,
    pub source: ExportSource,
    pub options: ExportOptions,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub rows: u64,
    pub bytes: u64,
    pub path: String,
    /// 准备连接时发生的事(断线重连过),要告诉用户。
    pub notice: Option<String>,
}

/// 为一次导入导出新开的专用连接用这个 id,和查询标签页的分开。
pub fn transfer_editor_id(job_id: &str) -> String {
    format!("transfer:{job_id}")
}

/// 重新执行一遍也不会改数据的语句才能为导出全部结果再跑一次。
/// 只看语句的写法:有副作用的函数(如取序列值)看不出来。
pub fn is_rerunnable(sql: &str) -> bool {
    const READS: [&str; 6] = ["SELECT", "WITH", "VALUES", "TABLE", "SHOW", "DESCRIBE"];
    const WRITES: [&str; 10] = [
        "INSERT", "UPDATE", "DELETE", "MERGE", "INTO", "CALL", "CREATE", "DROP", "ALTER",
        "TRUNCATE",
    ];
    let keyword = leading_keyword(sql);
    (READS.contains(&keyword.as_str()) || keyword == "DESC")
        && !bare_words(sql).iter().any(|w| WRITES.contains(&w.as_str()))
}

/// 语句里不在字符串、带引号的标识符和注释里的单词(大写)。
fn bare_words(sql: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = sql.chars().peekable();
    let mut word = String::new();
    let flush = |word: &mut String, words: &mut Vec<String>| {
        if !word.is_empty() {
            words.push(std::mem::take(word).to_ascii_uppercase());
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' | '`' => {
                flush(&mut word, &mut words);
                while let Some(next) = chars.next() {
                    if next == '\\' && c == '\'' {
                        chars.next();
                    } else if next == c {
                        break;
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                flush(&mut word, &mut words);
                chars.by_ref().find(|&n| n == '\n');
            }
            '/' if chars.peek() == Some(&'*') => {
                flush(&mut word, &mut words);
                chars.next();
                let mut previous = ' ';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
            }
            c if c.is_alphanumeric() || c == '_' => word.push(c),
            _ => flush(&mut word, &mut words),
        }
    }
    flush(&mut word, &mut words);
    words
}

/// 读到的行交给写器;第一次知道列时才写文件头(查询结果的列要等读到第一行)。
pub(super) struct WriterSink<'a> {
    writer: Box<dyn RowWriter>,
    rows: u64,
    cancel: &'a CancelFlag,
    throttle: Throttle<'a>,
}

impl<'a> WriterSink<'a> {
    pub(super) fn new(
        writer: Box<dyn RowWriter>,
        cancel: &'a CancelFlag,
        throttle: Throttle<'a>,
    ) -> Self {
        Self {
            writer,
            rows: 0,
            cancel,
            throttle,
        }
    }

    pub(super) fn begin(&mut self, columns: &[ExportColumn]) -> CoreResult<()> {
        self.writer.begin(columns)
    }

    /// 写文件尾、改名成目标文件,返回行数与文件大小。取消了就不落盘。
    pub(super) fn finish(self, output: OutputFile) -> CoreResult<(u64, u64)> {
        if self.cancel.is_cancelled() {
            return Err(CoreError::Cancelled);
        }
        let Self {
            mut writer,
            mut throttle,
            rows,
            ..
        } = self;
        throttle.force(&Progress::new(Phase::Finishing, rows));
        writer.finish()?;
        drop(writer);
        let bytes = output.commit()?;
        Ok((rows, bytes))
    }
}

impl RowSink for WriterSink<'_> {
    fn columns(&mut self, columns: &[ResultColumn]) -> CoreResult<()> {
        let columns: Vec<ExportColumn> = columns
            .iter()
            .map(|c| ExportColumn::new(&c.name, &c.data_type))
            .collect();
        self.begin(&columns)
    }

    fn row(&mut self, row: Vec<CellValue>) -> CoreResult<()> {
        self.writer.write_row(&row)?;
        self.rows += 1;
        self.throttle
            .report(&Progress::new(Phase::Exporting, self.rows));
        Ok(())
    }

    fn should_stop(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// 执行一次导出。取消时返回 [`CoreError::Cancelled`],不留下文件。
pub async fn run_export(
    sessions: &SessionManager,
    job_id: &str,
    request: &ExportRequest,
    cancel: &CancelFlag,
    progress: &dyn ProgressSink,
) -> CoreResult<ExportOutcome> {
    let mut throttle = Throttle::new(progress);
    throttle.force(&Progress::new(Phase::Preparing, 0));
    let session = sessions.get(&request.connection_id).await?;
    let dialect = session.dialect();
    let output = OutputFile::new(Path::new(&request.path))?;
    let sheet = match &request.source {
        ExportSource::Table { table, .. } => table.table.clone(),
        _ => "查询结果".to_owned(),
    };
    if let ExportFormat::Sql { table, .. } = &request.options.format
        && table.trim().is_empty()
    {
        return Err(CoreError::InvalidData(
            "请填写 INSERT 语句里的表名。".into(),
        ));
    }
    let writer = open_writer(&output, &request.options, dialect, &sheet)?;
    let mut sink = WriterSink::new(writer, cancel, throttle);
    let notice = match &request.source {
        ExportSource::Rows { columns, rows } => {
            sink.columns(columns)?;
            for row in rows {
                if cancel.is_cancelled() {
                    return Err(CoreError::Cancelled);
                }
                sink.row(row.clone())?;
            }
            None
        }
        ExportSource::Table {
            table,
            filter,
            sort,
        } => {
            let source = TableSource {
                connection_id: &request.connection_id,
                table,
                filter: filter.as_deref(),
                sort: sort.as_ref(),
                setup: None,
            };
            stream_table(sessions, job_id, &source, &mut sink).await?
        }
        ExportSource::Query {
            editor_id,
            scope,
            sql,
        } => {
            if !is_rerunnable(sql) {
                return Err(CoreError::InvalidData(
                    "这条语句会改数据或建对象，重新执行会再做一次，所以只能导出已经显示的结果；结果超出了显示上限，没法导出全部。"
                        .into(),
                ));
            }
            let reader = Reader {
                editor_id,
                connection_id: &request.connection_id,
                scope,
                setup: None,
            };
            stream(sessions, &reader, StreamQuery::Text(sql), &mut sink).await?
        }
    };
    let (rows, bytes) = sink.finish(output)?;
    Ok(ExportOutcome {
        rows,
        bytes,
        path: request.path.clone(),
        notice,
    })
}

/// 要整张读出的表(或筛选、排序后的结果)。
pub(super) struct TableSource<'a> {
    pub connection_id: &'a str,
    pub table: &'a TableRef,
    pub filter: Option<&'a str>,
    pub sort: Option<&'a SortSpec>,
    /// 读之前先在这条专用连接上执行的设置(转储 MySQL 时把时区设成 +00:00)。
    pub setup: Option<&'a str>,
}

/// 按表的列写文件头,再在这次任务的专用连接上把行流式读给 `sink`,读完关掉连接。返回准备连接时的提示。
pub(super) async fn stream_table(
    sessions: &SessionManager,
    job_id: &str,
    source: &TableSource<'_>,
    sink: &mut WriterSink<'_>,
) -> CoreResult<Option<String>> {
    let session = sessions.get(source.connection_id).await?;
    let table = source.table;
    let columns = session.table_columns(table).await?;
    let sql = select_all(
        session.dialect(),
        table,
        &columns,
        source.filter,
        source.sort,
    )?;
    let export: Vec<ExportColumn> = columns
        .iter()
        .map(|c| ExportColumn {
            is_generated: c.is_generated,
            ..ExportColumn::new(&c.name, &c.data_type)
        })
        .collect();
    sink.begin(&export)?;
    let editor = transfer_editor_id(job_id);
    let scope = QueryScope {
        database: Some(table.database.clone()),
        schema: None,
    };
    let query = StreamQuery::Table {
        sql: &sql,
        columns: &columns,
    };
    let reader = Reader {
        editor_id: &editor,
        connection_id: source.connection_id,
        scope: &scope,
        setup: source.setup,
    };
    let result = stream(sessions, &reader, query, sink).await;
    sessions.close_editor(&editor).await;
    result
}

/// 借连接、流式读完、还连接。返回准备连接时的提示。
/// 在查询标签页自己的连接上重新执行时,中途停下(取消、写不出来)要让数据库中止语句。PostgreSQL 里
/// 这会让标签页开着的事务整个失效,之后的 COMMIT 只会悄悄变成回滚——所以先试着开一个保存点,
/// 停下了就回到保存点。不在事务里时 SAVEPOINT 会报错,那也没关系,本来就没有事务可坏。
const EXPORT_SAVEPOINT: &str = "basalt_export";

/// 在哪条连接上读:查询标签页自己的,或这次任务新开的专用连接。
struct Reader<'a> {
    editor_id: &'a str,
    connection_id: &'a str,
    scope: &'a QueryScope,
    setup: Option<&'a str>,
}

fn exec_error(error: ExecError) -> CoreError {
    match error {
        ExecError::Cancelled => CoreError::Cancelled,
        ExecError::Lost(message) | ExecError::Rejected(message) => CoreError::Database(message),
    }
}

async fn stream(
    sessions: &SessionManager,
    reader: &Reader<'_>,
    query: StreamQuery<'_>,
    sink: &mut WriterSink<'_>,
) -> CoreResult<Option<String>> {
    if sink.cancel.is_cancelled() {
        return Err(CoreError::Cancelled);
    }
    let mut lease = sessions
        .lease_editor(reader.editor_id, reader.connection_id, reader.scope)
        .await?;
    let notice = lease.notice().map(str::to_owned);
    if let Some(setup) = reader.setup
        && let Err(error) = lease.execute_sql(setup).await
    {
        lease.release().await;
        return Err(exec_error(error));
    }
    let guard = matches!(query, StreamQuery::Text(_))
        && lease.session().dialect() == Dialect::Postgres
        && lease
            .execute_sql(&format!("SAVEPOINT {EXPORT_SAVEPOINT}"))
            .await
            .is_ok();
    let ended = lease.stream(query, sink).await;
    if guard {
        let finished = matches!(ended, Ok(StreamEnd::Finished { .. }));
        let steps = [
            (!finished).then(|| format!("ROLLBACK TO SAVEPOINT {EXPORT_SAVEPOINT}")),
            Some(format!("RELEASE SAVEPOINT {EXPORT_SAVEPOINT}")),
        ];
        for sql in steps.into_iter().flatten() {
            if let Err(error) = lease.cleanup_sql(&sql).await {
                log::warn!("导出后恢复查询标签页的事务失败：{error:?}");
            }
        }
    }
    lease.release().await;
    match ended? {
        StreamEnd::Finished { .. } => Ok(notice),
        StreamEnd::Cancelled => Err(CoreError::Cancelled),
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
