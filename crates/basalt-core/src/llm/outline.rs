//! 给 SQL 助手的表结构概要:一个库(PostgreSQL 是 schema)里每张表、视图的列名、类型、主键和注释。
//! 每家一条查询取全,不逐表去查;只有结构,没有数据。提示词怎么取舍由前端决定。

use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::session::Session;

/// 最多取这么多列:几万列的库,提示词里也放不下。
pub const MAX_COLUMNS: i64 = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineColumn {
    pub name: String,
    pub data_type: String,
    pub primary_key: bool,
    pub nullable: bool,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineTable {
    pub name: String,
    pub is_view: bool,
    pub comment: Option<String>,
    pub columns: Vec<OutlineColumn>,
}

/// 按表分组一行一列的查询结果:行已经按表名、列顺序排好。
pub(crate) fn group(rows: Vec<(String, bool, Option<String>, OutlineColumn)>) -> Vec<OutlineTable> {
    let mut tables: Vec<OutlineTable> = Vec::new();
    for (table, is_view, comment, column) in rows {
        match tables.last_mut() {
            Some(last) if last.name == table => last.columns.push(column),
            _ => tables.push(OutlineTable {
                name: table,
                is_view,
                comment: comment.filter(|c| !c.trim().is_empty()),
                columns: vec![column],
            }),
        }
    }
    tables
}

impl Session {
    pub async fn schema_outline(
        &self,
        database: &str,
        schema: Option<&str>,
    ) -> CoreResult<Vec<OutlineTable>> {
        match self {
            Self::Mysql(s) => s.schema_outline(database).await,
            Self::Postgres(s) => {
                let schema = schema
                    .ok_or_else(|| CoreError::Unsupported("PostgreSQL 需要指定 schema".into()))?;
                s.schema_outline(database, schema).await
            }
            Self::Sqlite(s) => s.schema_outline(database).await,
        }
    }
}
