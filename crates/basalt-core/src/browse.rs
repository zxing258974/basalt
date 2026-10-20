//! 表数据的读取与提交:把列信息、SQL 生成和各驱动的执行串起来。

use crate::driver::table_not_found;
use crate::error::{CoreError, CoreResult};
use crate::session::Session;
use crate::sql::Dialect;
use crate::sql::dml::{Statement, build_changes};
use crate::sql::select::select_page;
use crate::table::{
    ApplyOutcome, ColumnInfo, PageRequest, RowChange, TablePage, TableRef, read_only_reason,
};

impl Session {
    pub fn dialect(&self) -> Dialect {
        match self {
            Self::Mysql(_) => Dialect::Mysql,
            Self::Postgres(_) => Dialect::Postgres,
            Self::Sqlite(_) => Dialect::Sqlite,
        }
    }

    pub async fn table_columns(&self, table: &TableRef) -> CoreResult<Vec<ColumnInfo>> {
        if table.table.is_empty() {
            return Err(table_not_found(""));
        }
        match self {
            Self::Mysql(s) => s.columns(table).await,
            Self::Postgres(s) => s.columns(table).await,
            Self::Sqlite(s) => s.columns(table).await,
        }
    }

    pub async fn read_table(
        &self,
        table: &TableRef,
        request: &PageRequest,
    ) -> CoreResult<TablePage> {
        let columns = self.table_columns(table).await?;
        let sql = select_page(self.dialect(), table, &columns, request)?;
        let mut rows = match self {
            Self::Mysql(s) => s.fetch_rows(&sql, &columns).await?,
            Self::Postgres(s) => s.fetch_rows(&table.database, &sql, &columns).await?,
            Self::Sqlite(s) => s.fetch_rows(&sql, &columns).await?,
        };
        let has_more = rows.len() > request.limit as usize;
        rows.truncate(request.limit as usize);
        Ok(TablePage {
            read_only_reason: read_only_reason(&columns),
            columns,
            rows,
            has_more,
            sql,
        })
    }

    /// 生成将要执行的语句,参数内联成字面量给人看。与 [`Self::apply_changes`]
    /// 用同一套生成逻辑,预览什么就执行什么。
    pub async fn preview_changes(
        &self,
        table: &TableRef,
        changes: &[RowChange],
    ) -> CoreResult<Vec<String>> {
        Ok(self
            .plan_changes(table, changes)
            .await?
            .iter()
            .map(Statement::preview)
            .collect())
    }

    /// 在一个事务里执行全部修改;任何一条失败或没有恰好命中一行就整体回滚。
    ///
    /// `expected` 是用户确认过的预览:重新生成的语句与它不同(预览之后表结构变了),
    /// 就拒绝执行,保证执行的就是看过的。
    pub async fn apply_changes(
        &self,
        table: &TableRef,
        changes: &[RowChange],
        expected: Option<&[String]>,
    ) -> CoreResult<ApplyOutcome> {
        let statements = self.plan_changes(table, changes).await?;
        if let Some(expected) = expected {
            let previews: Vec<String> = statements.iter().map(Statement::preview).collect();
            if previews != expected {
                return Err(CoreError::InvalidEdit(
                    "表结构在预览之后变了，要执行的 SQL 和预览的不一样。请重新预览后再提交。"
                        .into(),
                ));
            }
        }
        if statements.is_empty() {
            return Ok(ApplyOutcome {
                statements: 0,
                affected_rows: 0,
            });
        }
        let affected_rows = match self {
            Self::Mysql(s) => s.execute(&statements).await?,
            Self::Postgres(s) => s.execute(&table.database, &statements).await?,
            Self::Sqlite(s) => s.execute(&statements).await?,
        };
        Ok(ApplyOutcome {
            statements: statements.len(),
            affected_rows,
        })
    }

    /// 每次都重新读列信息,不信任前端手里可能过期的表结构。
    async fn plan_changes(
        &self,
        table: &TableRef,
        changes: &[RowChange],
    ) -> CoreResult<Vec<Statement>> {
        let columns = self.table_columns(table).await?;
        let statements = build_changes(self.dialect(), table, &columns, changes)?;
        if statements.len() > 1
            && let Self::Mysql(session) = self
            && let Some(engine) = session.table_engine(table).await?
            && !is_transactional(&engine)
        {
            return Err(CoreError::InvalidEdit(format!(
                "这张表用的是 {engine} 引擎，不支持事务：一批修改中途失败时，前面已执行的无法回滚。请一次只提交一行修改。"
            )));
        }
        Ok(statements)
    }
}

/// MySQL / MariaDB 里支持事务的存储引擎。
fn is_transactional(engine: &str) -> bool {
    ["InnoDB", "NDB", "NDBCLUSTER", "RocksDB", "TokuDB"]
        .iter()
        .any(|known| known.eq_ignore_ascii_case(engine))
}

#[cfg(test)]
mod tests {
    use super::is_transactional;

    #[test]
    fn only_transactional_engines_can_take_a_batch() {
        assert!(is_transactional("InnoDB"));
        assert!(is_transactional("innodb"));
        assert!(!is_transactional("MyISAM"));
        assert!(!is_transactional("MEMORY"));
        assert!(!is_transactional("Aria"));
    }
}
