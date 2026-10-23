//! 导入导出在专用连接上的流式读取与语句执行(见 [`crate::query::lease`])。
//!
//! 读:一行一行解码后交给 sink,不攒在内存里。中途要停(用户取消、写文件出错)时先让数据库中止
//! 语句,再把剩下的结果读完丢掉,连接才能接着用。SQLite 执行期间装 progress handler,
//! 取消标志一亮就中止当前语句。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::{Stream, StreamExt};
use sqlx::mysql::MySqlRow;
use sqlx::postgres::{PgQueryResult, PgRow};
use sqlx::sqlite::{SqliteConnection, SqliteQueryResult, SqliteRow};
use sqlx::{Either, Executor, Row};

use super::editor::{CancelHandle, EditorConnection};
use super::sqlite_query::PROGRESS_STEPS;
use super::{mysql_data, mysql_query, postgres_data, postgres_query, sqlite_data};
use crate::query::collect::columns_of;
use crate::query::lease::{Driven, RowSink, StreamQuery};
use crate::sql::dml::Statement;
use crate::sql::dynamic;
use crate::table::ValueType;
use crate::value::CellValue;

type Decode<R> = fn(&R, usize, &[ValueType]) -> CellValue;

fn mysql_text(row: &MySqlRow, index: usize, _: &[ValueType]) -> CellValue {
    mysql_query::decode_text_cell(row, index)
}

fn mysql_typed(row: &MySqlRow, index: usize, types: &[ValueType]) -> CellValue {
    mysql_data::decode_cell(row, index, types.get(index).copied().unwrap_or_default())
}

fn pg_text(row: &PgRow, index: usize, _: &[ValueType]) -> CellValue {
    postgres_query::decode_text_cell(row, index)
}

fn pg_typed(row: &PgRow, index: usize, _: &[ValueType]) -> CellValue {
    postgres_data::decode_cell(row, index)
}

fn sqlite_cell(row: &SqliteRow, index: usize, _: &[ValueType]) -> CellValue {
    sqlite_data::decode_cell(row, index)
}

fn right<Q, R>(item: Result<R, sqlx::Error>) -> Result<Either<Q, R>, sqlx::Error> {
    item.map(Either::Right)
}

/// 读完一个流。只要第一个结果集;要停时让数据库中止语句,剩下的读完丢掉。
async fn drive<S, Q, R>(
    mut stream: S,
    decode: Decode<R>,
    types: &[ValueType],
    sink: &mut dyn RowSink,
    cancel: &CancelHandle,
    requested: &AtomicBool,
    known_columns: bool,
) -> Driven
where
    S: Stream<Item = Result<Either<Q, R>, sqlx::Error>> + Unpin,
    R: Row,
{
    let mut driven = Driven {
        rows: 0,
        columns_seen: known_columns,
        stop: None,
        error: None,
    };
    let mut first_set_done = false;
    while let Some(item) = stream.next().await {
        let row = match item {
            Ok(Either::Right(row)) => row,
            Ok(Either::Left(_)) => {
                first_set_done = first_set_done || driven.rows > 0 || driven.columns_seen;
                continue;
            }
            Err(error) => {
                driven.error = Some(error);
                break;
            }
        };
        if driven.stop.is_some() || first_set_done {
            continue;
        }
        let mut result = Ok(());
        if !driven.columns_seen {
            driven.columns_seen = true;
            result = sink.columns(&columns_of(&row));
        }
        if result.is_ok() {
            let width = row.columns().len();
            result = sink.row((0..width).map(|i| decode(&row, i, types)).collect());
        }
        let stop = match result {
            Err(error) => Some(Err(error)),
            Ok(()) => {
                driven.rows += 1;
                (sink.should_stop() || requested.load(Ordering::Relaxed)).then_some(Ok(()))
            }
        };
        if let Some(stop) = stop {
            driven.stop = Some(stop);
            if let Err(error) = cancel.cancel().await {
                log::warn!("中止导出的语句失败，只能等它自己读完：{error}");
            }
        }
    }
    driven
}

fn sqlite_flag(cancel: &CancelHandle) -> Arc<AtomicBool> {
    match cancel {
        CancelHandle::Sqlite { flag } => flag.clone(),
        _ => Arc::new(AtomicBool::new(false)),
    }
}

/// 执行期间装上 progress handler,标志一亮就让 SQLite 中止当前语句。
async fn watch_sqlite(
    conn: &mut SqliteConnection,
    flag: Arc<AtomicBool>,
) -> Result<(), sqlx::Error> {
    let mut handle = conn.lock_handle().await?;
    handle.set_progress_handler(PROGRESS_STEPS, move || !flag.load(Ordering::Relaxed));
    Ok(())
}

async fn unwatch_sqlite(conn: &mut SqliteConnection) {
    match conn.lock_handle().await {
        Ok(mut handle) => handle.remove_progress_handler(),
        Err(error) => log::warn!("移除 SQLite progress handler 失败：{error}"),
    }
}

fn failed(error: sqlx::Error) -> Driven {
    Driven {
        rows: 0,
        columns_seen: false,
        stop: None,
        error: Some(error),
    }
}

impl EditorConnection {
    pub(crate) async fn stream(
        &mut self,
        query: &StreamQuery<'_>,
        cancel: &CancelHandle,
        requested: &AtomicBool,
        sink: &mut dyn RowSink,
    ) -> Driven {
        let (sql, types, typed): (&str, Vec<ValueType>, bool) = match query {
            StreamQuery::Text(sql) => (sql, Vec::new(), false),
            StreamQuery::Table { sql, columns } => {
                (sql, columns.iter().map(|c| c.value_type).collect(), true)
            }
        };
        match self {
            // 和表数据页一样走文本协议:预备语句读视图可能次次报 1615(见 MysqlSession::fetch_rows)。
            Self::Mysql(editor) if typed => {
                let stream = sqlx::raw_sql(dynamic(sql)).fetch_many(&mut editor.conn);
                drive(stream, mysql_typed, &types, sink, cancel, requested, true).await
            }
            Self::Mysql(editor) => {
                let stream = sqlx::raw_sql(dynamic(sql)).fetch_many(&mut editor.conn);
                drive(stream, mysql_text, &types, sink, cancel, requested, false).await
            }
            Self::Postgres(editor) if typed => {
                let stream = sqlx::query(dynamic(sql))
                    .persistent(false)
                    .fetch(&mut editor.conn)
                    .map(right::<PgQueryResult, PgRow>);
                drive(stream, pg_typed, &types, sink, cancel, requested, true).await
            }
            Self::Postgres(editor) => {
                let stream = sqlx::raw_sql(dynamic(sql)).fetch_many(&mut editor.conn);
                drive(stream, pg_text, &types, sink, cancel, requested, false).await
            }
            Self::Sqlite(editor) => {
                if let Err(error) = watch_sqlite(&mut editor.conn, sqlite_flag(cancel)).await {
                    return failed(error);
                }
                let driven = if typed {
                    let stream = sqlx::query(dynamic(sql))
                        .persistent(false)
                        .fetch(&mut editor.conn)
                        .map(right::<SqliteQueryResult, SqliteRow>);
                    drive(stream, sqlite_cell, &types, sink, cancel, requested, true).await
                } else {
                    let stream = sqlx::raw_sql(dynamic(sql)).fetch_many(&mut editor.conn);
                    drive(stream, sqlite_cell, &types, sink, cancel, requested, false).await
                };
                unwatch_sqlite(&mut editor.conn).await;
                driven
            }
        }
    }

    /// 执行一条参数化语句。预备语句按 SQL 文本缓存:导入时满批的语句都一样,只预备一次。
    pub(crate) async fn execute_statement(
        &mut self,
        statement: &Statement,
        cancel: &CancelHandle,
    ) -> Result<u64, sqlx::Error> {
        let sql = statement.sql();
        let params = statement.params();
        match self {
            Self::Mysql(editor) => {
                let query = params
                    .into_iter()
                    .fold(sqlx::query(dynamic(&sql)), mysql_data::bind);
                Ok(query.execute(&mut editor.conn).await?.rows_affected())
            }
            Self::Postgres(editor) => {
                let query = params
                    .into_iter()
                    .fold(sqlx::query(dynamic(&sql)), postgres_data::bind);
                Ok(query.execute(&mut editor.conn).await?.rows_affected())
            }
            Self::Sqlite(editor) => {
                watch_sqlite(&mut editor.conn, sqlite_flag(cancel)).await?;
                let query = params
                    .into_iter()
                    .fold(sqlx::query(dynamic(&sql)), sqlite_data::bind);
                let result = query.execute(&mut editor.conn).await;
                unwatch_sqlite(&mut editor.conn).await;
                Ok(result?.rows_affected())
            }
        }
    }

    /// 执行一条不带参数的语句,走文本协议(BEGIN、SAVEPOINT 这类语句有的库不能预备)。
    /// `cancel` 为空时 SQLite 不装取消检查(收尾的 ROLLBACK 不能被取消标志打断)。
    pub(crate) async fn execute_sql(
        &mut self,
        sql: &str,
        cancel: Option<&CancelHandle>,
    ) -> Result<u64, sqlx::Error> {
        match self {
            Self::Mysql(editor) => Ok((&mut editor.conn)
                .execute(dynamic(sql))
                .await?
                .rows_affected()),
            Self::Postgres(editor) => Ok((&mut editor.conn)
                .execute(dynamic(sql))
                .await?
                .rows_affected()),
            Self::Sqlite(editor) if cancel.is_none() => Ok((&mut editor.conn)
                .execute(dynamic(sql))
                .await?
                .rows_affected()),
            Self::Sqlite(editor) => {
                let flag = cancel.map(sqlite_flag).unwrap_or_default();
                watch_sqlite(&mut editor.conn, flag).await?;
                let result = (&mut editor.conn).execute(dynamic(sql)).await;
                unwatch_sqlite(&mut editor.conn).await;
                Ok(result?.rows_affected())
            }
        }
    }
}
