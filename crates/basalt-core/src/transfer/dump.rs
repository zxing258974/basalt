//! 转储 SQL 文件:一张表的建表语句,连同(或不带)全部数据的 INSERT,还原时从头执行一遍即可。
//!
//! 建表语句和信息面板同一个来源(见 [`crate::table_info::CreateScript`]);数据走导出同一套流式读取、
//! SQL INSERT 写器、进度与取消。开头先删掉同名表(DROP TABLE IF EXISTS),和 Navicat、mysqldump 一样。
//! - MySQL:SHOW CREATE TABLE 里的表名不带库名,INSERT 也不带,还原进哪个库看执行时的当前库;
//!   前后关、开外键检查,被引用的表还没建也建得出来。
//! - PostgreSQL:建表语句按表结构生成,带 schema。serial 列的序列不在建表语句里:先 CREATE SEQUENCE IF NOT EXISTS,
//!   建表后再 OWNED BY 回这一列;带数据时最后把标识列、serial 的序列推到已有的最大值。
//! - SQLite:`sqlite_master` 里的原文;触发器放到数据之后,免得还原时每插一行就触发一次。

use std::io::BufWriter;
use std::path::Path;

use serde::Deserialize;

use super::export::{ExportOutcome, TableSource, WriterSink, stream_table};
use super::progress::{CancelFlag, Phase, Progress, ProgressSink, Throttle};
use super::sql_write::{SqlFrame, SqlWriter};
use super::writer::{BUFFER_BYTES, OutputFile};
use crate::error::CoreResult;
use crate::session::{Session, SessionManager};
use crate::sql::{Dialect, quote_ident, table_name};
use crate::table::TableRef;
use crate::table_info::{CreateScript, join_statements};
use crate::table_ops::reset_identity_sql;

/// 每条 INSERT 合几行,和导出对话框的默认值一样。
const DUMP_ROWS_PER_STATEMENT: u32 = 100;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRequest {
    pub connection_id: String,
    pub table: TableRef,
    /// 为假时只写建表语句。
    pub with_data: bool,
    pub path: String,
}

/// PostgreSQL 里带序列的列:标识列,或默认值取自它名下序列的 serial 列。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceColumn {
    pub column: String,
    /// `pg_get_serial_sequence` 给的名字,已经带 schema、该加引号的加好了。
    pub sequence: String,
    pub identity: bool,
}

/// 写进注释的名字:去掉换行等控制字符,免得名字里的换行把后面的内容变成可执行的语句。
fn comment_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// INSERT 写进哪张表(加好引号),以及写在 INSERT 前后的内容。
pub fn dump_frame(
    dialect: Dialect,
    table: &TableRef,
    script: &CreateScript,
    sequences: &[SequenceColumn],
    with_data: bool,
) -> (String, SqlFrame) {
    let target = match dialect {
        Dialect::Postgres => table_name(dialect, table),
        Dialect::Mysql | Dialect::Sqlite => quote_ident(dialect, &table.table),
    };
    let serials: Vec<&SequenceColumn> = sequences.iter().filter(|s| !s.identity).collect();
    let mut create = vec![format!("DROP TABLE IF EXISTS {target}")];
    create.extend(
        serials
            .iter()
            .map(|s| format!("CREATE SEQUENCE IF NOT EXISTS {}", s.sequence)),
    );
    create.extend(script.before.iter().cloned());
    create.extend(serials.iter().map(|s| {
        format!(
            "ALTER SEQUENCE {} OWNED BY {target}.{}",
            s.sequence,
            quote_ident(dialect, &s.column)
        )
    }));
    let mut before = String::new();
    if dialect == Dialect::Mysql {
        before.push_str("SET FOREIGN_KEY_CHECKS = 0;\n\n");
    }
    before.push_str("-- 表结构\n");
    before.push_str(&join_statements(create));
    before.push_str("\n\n");
    if with_data {
        before.push_str("-- 数据\n");
    }

    let mut after: Vec<String> = Vec::new();
    if with_data && dialect == Dialect::Postgres {
        after.extend(
            sequences
                .iter()
                .map(|s| reset_identity_sql(table, &s.column)),
        );
    }
    after.extend(script.after.iter().cloned());
    if dialect == Dialect::Mysql {
        after.push("SET FOREIGN_KEY_CHECKS = 1".into());
    }
    let after = if after.is_empty() {
        String::new()
    } else {
        format!("{}\n", join_statements(after))
    };
    let name = [
        Some(&table.database),
        table.schema.as_ref(),
        Some(&table.table),
    ]
    .into_iter()
    .flatten()
    .map(String::as_str)
    .collect::<Vec<_>>()
    .join(".");
    let frame = SqlFrame {
        title: format!("由 Basalt 转储：{}", comment_text(&name)),
        before,
        after,
    };
    (target, frame)
}

/// 执行一次转储。取消时返回 [`crate::CoreError::Cancelled`],不留下文件。
pub async fn run_dump(
    sessions: &SessionManager,
    job_id: &str,
    request: &DumpRequest,
    cancel: &CancelFlag,
    progress: &dyn ProgressSink,
) -> CoreResult<ExportOutcome> {
    let mut throttle = Throttle::new(progress);
    throttle.force(&Progress::new(Phase::Preparing, 0));
    let session = sessions.get(&request.connection_id).await?;
    let table = &request.table;
    let script = session.create_script(table).await?;
    let sequences = match &*session {
        Session::Postgres(s) => s.sequence_columns(table).await?,
        Session::Mysql(_) | Session::Sqlite(_) => Vec::new(),
    };
    let dialect = session.dialect();
    let (target, frame) = dump_frame(dialect, table, &script, &sequences, request.with_data);
    let output = OutputFile::new(Path::new(&request.path))?;
    let file = BufWriter::with_capacity(BUFFER_BYTES, output.create()?);
    let writer = SqlWriter::framed(file, dialect, target, DUMP_ROWS_PER_STATEMENT, frame);
    let mut sink = WriterSink::new(Box::new(writer), cancel, throttle);
    let notice = if request.with_data {
        let source = TableSource {
            connection_id: &request.connection_id,
            table,
            filter: None,
            sort: None,
        };
        stream_table(sessions, job_id, &source, &mut sink).await?
    } else {
        sink.begin(&[])?;
        None
    };
    let (rows, bytes) = sink.finish(output)?;
    Ok(ExportOutcome {
        rows,
        bytes,
        path: request.path.clone(),
        notice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(dialect: Dialect, name: &str) -> TableRef {
        TableRef {
            database: "shop".into(),
            schema: (dialect == Dialect::Postgres).then(|| "public".into()),
            table: name.into(),
        }
    }

    fn script(before: &[&str], after: &[&str]) -> CreateScript {
        CreateScript {
            before: before.iter().map(|s| (*s).to_owned()).collect(),
            after: after.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn mysql_turns_foreign_key_checks_off_around_the_dump() {
        let my = Dialect::Mysql;
        let (target, frame) = dump_frame(
            my,
            &table(my, "users"),
            &script(&["CREATE TABLE `users` (\n  `id` int\n)"], &[]),
            &[],
            true,
        );
        assert_eq!(target, "`users`");
        assert_eq!(frame.title, "由 Basalt 转储：shop.users");
        assert_eq!(
            frame.before,
            "SET FOREIGN_KEY_CHECKS = 0;\n\n-- 表结构\nDROP TABLE IF EXISTS `users`;\n\n\
             CREATE TABLE `users` (\n  `id` int\n);\n\n-- 数据\n"
        );
        assert_eq!(frame.after, "SET FOREIGN_KEY_CHECKS = 1;\n");
    }

    #[test]
    fn postgres_recreates_serial_sequences_and_moves_them_past_the_data() {
        let pg = Dialect::Postgres;
        let sequences = [
            SequenceColumn {
                column: "id".into(),
                sequence: "public.users_id_seq".into(),
                identity: false,
            },
            SequenceColumn {
                column: "No".into(),
                sequence: "public.\"users_No_seq\"".into(),
                identity: true,
            },
        ];
        let create = script(&["CREATE TABLE \"public\".\"users\" (\"id\" integer)"], &[]);
        let users = table(pg, "users");
        let (target, frame) = dump_frame(pg, &users, &create, &sequences, true);
        assert_eq!(target, "\"public\".\"users\"");
        assert_eq!(
            frame.before,
            "-- 表结构\nDROP TABLE IF EXISTS \"public\".\"users\";\n\n\
             CREATE SEQUENCE IF NOT EXISTS public.users_id_seq;\n\n\
             CREATE TABLE \"public\".\"users\" (\"id\" integer);\n\n\
             ALTER SEQUENCE public.users_id_seq OWNED BY \"public\".\"users\".\"id\";\n\n-- 数据\n"
        );
        assert_eq!(
            frame.after,
            format!(
                "{};\n\n{};\n",
                reset_identity_sql(&users, "id"),
                reset_identity_sql(&users, "No")
            )
        );
        // 只要结构:序列照样建,不推序列。
        let (_, shape) = dump_frame(pg, &users, &create, &sequences, false);
        assert!(shape.before.ends_with("\"id\";\n\n"), "{}", shape.before);
        assert_eq!(shape.after, "");
    }

    #[test]
    fn sqlite_creates_triggers_after_the_data() {
        let lite = Dialect::Sqlite;
        let (target, frame) = dump_frame(
            lite,
            &table(lite, "notes"),
            &script(
                &[
                    "CREATE TABLE notes (id INTEGER)",
                    "CREATE INDEX n ON notes (id)",
                ],
                &["CREATE TRIGGER t AFTER INSERT ON notes BEGIN SELECT 1; END"],
            ),
            &[],
            false,
        );
        assert_eq!(target, "\"notes\"");
        assert_eq!(
            frame.before,
            "-- 表结构\nDROP TABLE IF EXISTS \"notes\";\n\nCREATE TABLE notes (id INTEGER);\n\n\
             CREATE INDEX n ON notes (id);\n\n"
        );
        assert_eq!(
            frame.after,
            "CREATE TRIGGER t AFTER INSERT ON notes BEGIN SELECT 1; END;\n"
        );
    }

    #[test]
    fn names_with_line_breaks_stay_inside_the_comment() {
        let lite = Dialect::Sqlite;
        let (_, frame) = dump_frame(
            lite,
            &table(lite, "x\nDROP TABLE y;--"),
            &CreateScript::default(),
            &[],
            false,
        );
        assert_eq!(frame.title, "由 Basalt 转储：shop.x DROP TABLE y;--");
        assert!(!frame.title.contains('\n'));
    }
}
