//! 导入时一批一批写进数据库:事务、保存点、MySQL 的按批提交。
//!
//! PostgreSQL、SQLite 整个导入在一个事务里,出错、取消整体回滚。MySQL 每批单独提交
//! (大事务会撑满 binlog 缓存、拖住 undo,MyISAM 这类引擎本来就没有事务),所以中途出错时
//! 前面已经提交的批次留在库里,报告里写清楚。
//!
//! 每批先整批写;数据库拒绝了就回到这批开头的保存点,再一行一行写,找出是哪几行,
//! 其余的照常写进去(遇错即停时找到第一行就停)。不支持事务的引擎一行一条语句,
//! 成功一行就生效一行。
//!
//! 追加、复制、追加或更新是多行 INSERT;更新、删除按匹配键一行一条语句。

use super::import::ImportMode;
use super::report::{Failures, ImportFailure};
use crate::query::lease::{EditorLease, ExecError};
use crate::sql::Dialect;
use crate::sql::bind::BindValue;
use crate::sql::dml::Statement;
use crate::sql::insert::{
    clear_table, delete_row, insert_rows, rows_per_statement, update_row, upsert_rows,
};
use crate::table::{ColumnInfo, TableRef};

/// 一批里的一行:文件行号与转换好的值。
pub(crate) type Row = (u64, Vec<BindValue>);

/// 导入为什么中途停了。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Abort {
    Cancelled,
    /// 遇错即停:第一条被数据库拒绝的行已经记进失败报告。
    Stopped,
    /// 连接断了。
    Lost(String),
    /// COMMIT 本身失败,不知道生效了没有。
    CommitFailed(String),
}

impl From<ExecError> for Abort {
    fn from(error: ExecError) -> Self {
        match error {
            ExecError::Cancelled => Self::Cancelled,
            ExecError::Lost(message) => Self::Lost(message),
            // BEGIN、SAVEPOINT 这类控制语句被拒绝,没法再往下写了。
            ExecError::Rejected(message) => Self::Lost(message),
        }
    }
}

pub(crate) struct BatchWriter<'a> {
    pub lease: &'a mut EditorLease,
    pub dialect: Dialect,
    pub table: &'a TableRef,
    pub columns: Vec<&'a ColumnInfo>,
    pub mode: ImportMode,
    /// 匹配键在 `columns` 里的位置(更新、追加或更新、删除)。
    pub keys: Vec<usize>,
    /// MySQL:每批提交。
    pub commit_each_batch: bool,
    /// MySQL 的 MyISAM、MEMORY 等不支持事务的引擎。
    pub transactional: bool,
    pub stop_on_error: bool,
    /// 插入、更新或删除的表行数(含还没提交的)。
    pub affected: u64,
    /// 在表里找不到对应行的文件行数(更新、删除),含还没提交的。
    pub unmatched: u64,
    /// 已经提交的行数、未匹配行数、批数,以及最后一批提交到文件的哪一行(MySQL 报告用)。
    pub committed: u64,
    pub committed_unmatched: u64,
    pub committed_batches: u64,
    pub committed_through: Option<u64>,
    /// 清空表时删掉的行数。
    pub cleared: Option<u64>,
}

impl BatchWriter<'_> {
    async fn sql(&mut self, sql: &str) -> Result<u64, ExecError> {
        self.lease.execute_sql(sql).await
    }

    /// 开事务;「复制」模式在同一个事务里先清空表。
    pub async fn begin(&mut self) -> Result<(), Abort> {
        if self.transactional {
            self.sql("BEGIN").await?;
        }
        if self.mode == ImportMode::Replace {
            let sql = clear_table(self.dialect, self.table);
            self.cleared = Some(self.sql(&sql).await?);
        }
        Ok(())
    }

    /// 写一批(或一行),返回 (生效的表行数, 没匹配上的文件行数)。
    async fn apply(&mut self, rows: &[Row]) -> Result<(u64, u64), ExecError> {
        match self.mode {
            ImportMode::Append | ImportMode::Replace | ImportMode::Upsert => {
                let per = rows_per_statement(self.dialect, self.columns.len(), rows.len());
                let mut affected = 0;
                for chunk in rows.chunks(per) {
                    let values = chunk.iter().map(|(_, values)| values.clone()).collect();
                    let statement = if self.mode == ImportMode::Upsert {
                        upsert_rows(self.dialect, self.table, &self.columns, &self.keys, values)
                    } else {
                        insert_rows(self.dialect, self.table, &self.columns, values)
                    };
                    let count = self.lease.execute(&statement).await?;
                    // MySQL 的 upsert 更新一行算 2;一行文件就算一行。
                    affected += if self.mode == ImportMode::Upsert {
                        chunk.len() as u64
                    } else {
                        count
                    };
                }
                Ok((affected, 0))
            }
            ImportMode::Update | ImportMode::Delete => {
                let (mut affected, mut unmatched) = (0, 0);
                for (_, values) in rows {
                    let statement = self.keyed(values);
                    match self.lease.execute(&statement).await? {
                        0 => unmatched += 1,
                        count => affected += count,
                    }
                }
                Ok((affected, unmatched))
            }
        }
    }

    /// 更新、删除的一行:按匹配键找到表里对应的行。
    fn keyed(&self, values: &[BindValue]) -> Statement {
        let pick = |key: bool| -> Vec<(&ColumnInfo, BindValue)> {
            self.columns
                .iter()
                .zip(values)
                .enumerate()
                .filter(|(index, _)| self.keys.contains(index) == key)
                .map(|(_, (column, value))| (*column, value.clone()))
                .collect()
        };
        match self.mode {
            ImportMode::Update => update_row(self.dialect, self.table, &pick(false), &pick(true)),
            _ => delete_row(self.dialect, self.table, &pick(true)),
        }
    }

    /// 写一批。被拒绝的行记进 `failures`;遇错即停时返回 [`Abort::Stopped`]。
    pub async fn write(&mut self, rows: Vec<Row>, failures: &mut Failures) -> Result<(), Abort> {
        if rows.is_empty() {
            return Ok(());
        }
        let whole = if self.transactional {
            self.sql("SAVEPOINT basalt_batch").await?;
            self.apply(&rows).await
        } else {
            Err(ExecError::Rejected(String::new()))
        };
        match whole {
            Ok((affected, unmatched)) => {
                self.sql("RELEASE SAVEPOINT basalt_batch").await?;
                self.affected += affected;
                self.unmatched += unmatched;
            }
            Err(ExecError::Rejected(_)) => {
                if self.transactional {
                    self.sql("ROLLBACK TO SAVEPOINT basalt_batch").await?;
                    self.sql("RELEASE SAVEPOINT basalt_batch").await?;
                }
                self.row_by_row(&rows, failures).await?;
            }
            Err(other) => return Err(other.into()),
        }
        if self.commit_each_batch {
            self.commit().await?;
            self.committed_through = rows.last().map(|(row, _)| *row);
            if self.transactional {
                self.sql("BEGIN").await?;
            }
        }
        Ok(())
    }

    async fn row_by_row(&mut self, rows: &[Row], failures: &mut Failures) -> Result<(), Abort> {
        for row in rows {
            if self.transactional {
                self.sql("SAVEPOINT basalt_row").await?;
            }
            match self.apply(std::slice::from_ref(row)).await {
                Ok((affected, unmatched)) => {
                    if self.transactional {
                        self.sql("RELEASE SAVEPOINT basalt_row").await?;
                    }
                    self.affected += affected;
                    self.unmatched += unmatched;
                }
                Err(ExecError::Rejected(reason)) => {
                    if self.transactional {
                        self.sql("ROLLBACK TO SAVEPOINT basalt_row").await?;
                        self.sql("RELEASE SAVEPOINT basalt_row").await?;
                    }
                    failures.push(ImportFailure {
                        row: row.0,
                        column: None,
                        value: None,
                        reason,
                    });
                    if self.stop_on_error {
                        return Err(Abort::Stopped);
                    }
                }
                Err(other) => return Err(other.into()),
            }
        }
        Ok(())
    }

    /// 提交不能被取消打断(被 KILL 的 COMMIT 可能已经生效);出了错一律说不清生效没有。
    async fn commit(&mut self) -> Result<(), Abort> {
        if self.transactional {
            self.lease
                .cleanup_sql("COMMIT")
                .await
                .map_err(|error| match error {
                    ExecError::Cancelled => Abort::CommitFailed("提交时被取消".into()),
                    ExecError::Lost(message) | ExecError::Rejected(message) => {
                        Abort::CommitFailed(message)
                    }
                })?;
        }
        self.committed = self.affected;
        self.committed_unmatched = self.unmatched;
        self.committed_batches += 1;
        Ok(())
    }

    /// 全部写完:提交。
    pub async fn finish(&mut self) -> Result<(), Abort> {
        self.commit().await
    }

    /// 出错或取消:撤回还没提交的。回滚失败只记日志,连接断开后数据库会自己回滚。
    pub async fn abandon(&mut self) {
        if self.transactional
            && let Err(error) = self.lease.cleanup_sql("ROLLBACK").await
        {
            log::warn!("导入回滚失败：{error:?}");
        }
        if !self.transactional {
            // 没有事务:写进去的就是生效了。
            self.committed = self.affected;
            self.committed_unmatched = self.unmatched;
        }
        self.affected = self.committed;
        self.unmatched = self.committed_unmatched;
    }
}
