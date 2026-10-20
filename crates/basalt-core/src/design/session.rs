//! Session 上的读表结构、预览 DDL、执行 DDL:把驱动的读取、DDL 生成和执行串起来。

use super::{DdlContext, DdlOutcome, DdlPlan, DdlPreview, ExecMode, TableDesign, plan};
use crate::driver::table_not_found;
use crate::error::{CoreError, CoreResult};
use crate::session::Session;
use crate::table::TableRef;

impl Session {
    pub async fn read_design(&self, table: &TableRef) -> CoreResult<TableDesign> {
        if table.table.is_empty() {
            return Err(table_not_found(""));
        }
        let mut design = match self {
            Self::Mysql(s) => s.read_design(table).await?,
            Self::Postgres(s) => s.read_design(table).await?,
            Self::Sqlite(s) => s.read_design(table).await?,
        };
        design.version = design.fingerprint();
        Ok(design)
    }

    /// 将要执行的 DDL。与 [`Self::apply_design`] 用同一套生成逻辑,预览什么就执行什么。
    /// `original` 是要修改的表,新建表时为 `None`。
    pub async fn preview_design(
        &self,
        original: Option<&TableRef>,
        target: &TableDesign,
    ) -> CoreResult<DdlPreview> {
        Ok(self.plan_design(original, target).await?.preview())
    }

    /// `expected` 是用户确认过的预览:重新生成的语句和它不一样(预览之后表结构变了)就拒绝执行。
    pub async fn apply_design(
        &self,
        original: Option<&TableRef>,
        target: &TableDesign,
        expected: Option<&[String]>,
    ) -> CoreResult<DdlOutcome> {
        let plan = self.plan_design(original, target).await?;
        if let Some(expected) = expected
            && plan.preview().statements != expected
        {
            return Err(CoreError::InvalidEdit(
                "表结构在预览之后变了，要执行的 SQL 和预览的不一样。请重新预览后再保存。".into(),
            ));
        }
        let statements = if plan.statements.is_empty() {
            0
        } else {
            match self {
                Self::Mysql(s) => s.execute_ddl(&plan.statements).await?,
                Self::Postgres(s) => {
                    s.execute_ddl(&target.table.database, &plan.statements)
                        .await?
                }
                Self::Sqlite(s) => {
                    s.execute_ddl(&plan.statements, plan.mode == ExecMode::Rebuild)
                        .await?
                }
            }
        };
        Ok(DdlOutcome {
            statements,
            table: target.table.clone(),
        })
    }

    /// 每次都重新读原结构,不信任前端手里可能过期的那份。
    async fn plan_design(
        &self,
        original: Option<&TableRef>,
        target: &TableDesign,
    ) -> CoreResult<DdlPlan> {
        if matches!(self, Self::Postgres(_)) && target.table.schema.is_none() {
            return Err(CoreError::Unsupported("PostgreSQL 需要指定 schema".into()));
        }
        let before = match original {
            Some(table) => {
                if table.database != target.table.database || table.schema != target.table.schema {
                    return Err(CoreError::InvalidEdit(
                        "不能把表挪到别的库或 schema。".into(),
                    ));
                }
                let fresh = self.read_design(table).await?;
                if fresh.version != target.version {
                    return Err(CoreError::InvalidEdit(
                        "这张表在你打开设计器之后被改过，草稿是按旧结构做的。请再保存一次（会先按最新结构重新合并你的修改）。"
                            .into(),
                    ));
                }
                Some(fresh)
            }
            None => None,
        };
        let mut context = DdlContext::new(self.dialect());
        match (self, original) {
            (Self::Mysql(s), _) => context.backslash_escapes = s.backslash_escapes().await?,
            (Self::Sqlite(s), Some(table)) => {
                context.schema_objects = s.schema_objects(table).await?;
                context.referencing_tables = s.referencing_tables(table).await?;
                context.rebuild_blockers = s.rebuild_blockers(table).await?;
            }
            _ => {}
        }
        plan(&context, before.as_ref(), target)
    }
}
