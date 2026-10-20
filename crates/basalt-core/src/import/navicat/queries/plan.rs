//! 导入预览与写入:对照已保存的查询给每条 Navicat 查询定状态,再写入勾选的「新增」项。

use serde::Serialize;

use super::{MatchedBy, NavicatQuery, QueryTarget, match_target, read_query};
use crate::config::ConnectionConfig;
use crate::import::SourceConnection;
use crate::import::navicat::display_type;
use crate::query::saved::SavedQueryStore;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueryImportStatus {
    New,
    /// 目标位置已经有内容一样的同名查询(多半之前导入过)。
    Exists,
    /// 目标位置已经有同名、内容不同的查询,不覆盖。
    Conflict,
    Unmatched {
        reason: String,
    },
}

/// 导入预览里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryImportCandidate {
    pub id: String,
    pub name: String,
    pub source_type: String,
    pub source_connection: String,
    pub source_database: String,
    pub source_schema: Option<String>,
    pub target: Option<QueryTarget>,
    pub matched_by: Option<MatchedBy>,
    pub status: QueryImportStatus,
}

/// 预览一行,连同要写入的内容(只在能导入时有)。
pub struct PlannedQuery {
    pub candidate: QueryImportCandidate,
    pub sql: Option<String>,
}

/// 对照 Basalt 的连接和已保存的查询,给每条 Navicat 查询定去向与状态。
pub fn plan_queries(
    queries: &[NavicatQuery],
    connections: &[ConnectionConfig],
    navicat: &[SourceConnection],
    store: &SavedQueryStore,
) -> Vec<PlannedQuery> {
    queries
        .iter()
        .map(|query| {
            let matched = match_target(query, connections, navicat);
            let (target, matched_by) = match &matched {
                Ok((target, by)) => (Some(target.clone()), Some(*by)),
                Err(_) => (None, None),
            };
            let (status, sql) = match matched {
                Err(reason) => (QueryImportStatus::Unmatched { reason }, None),
                Ok((target, _)) => status_for(query, &target, store),
            };
            PlannedQuery {
                candidate: QueryImportCandidate {
                    id: query.id.clone(),
                    name: query.name.clone(),
                    source_type: display_type(&query.conn_type),
                    source_connection: query.connection.clone(),
                    source_database: query.database.clone(),
                    source_schema: query.schema.clone(),
                    target,
                    matched_by,
                    status,
                },
                sql,
            }
        })
        .collect()
}

fn status_for(
    query: &NavicatQuery,
    target: &QueryTarget,
    store: &SavedQueryStore,
) -> (QueryImportStatus, Option<String>) {
    let sql = match read_query(query) {
        Ok(sql) => sql,
        Err(reason) => return (QueryImportStatus::Unmatched { reason }, None),
    };
    let saved = target.saved_ref();
    if !store.exists(&saved) {
        return (QueryImportStatus::New, Some(sql));
    }
    match store.read(&saved) {
        Ok(existing) if existing == sql => (QueryImportStatus::Exists, None),
        _ => (QueryImportStatus::Conflict, None),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedQuery {
    pub name: String,
    pub target: QueryTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedQuery {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryImportReport {
    pub imported: Vec<ImportedQuery>,
    pub skipped: Vec<SkippedQuery>,
}

/// 写入勾选的、状态为「新增」的查询。不覆盖任何已有文件;单条写不进去只记进「跳过」。
pub fn import_planned(
    plans: &[PlannedQuery],
    ids: &[String],
    store: &SavedQueryStore,
) -> QueryImportReport {
    let mut report = QueryImportReport::default();
    for id in ids {
        let Some(plan) = plans.iter().find(|p| &p.candidate.id == id) else {
            report.skipped.push(SkippedQuery {
                name: id.clone(),
                reason: "扫描之后 Navicat 的文件有变化，找不到这条查询，请重新扫描".into(),
            });
            continue;
        };
        let candidate = &plan.candidate;
        let skip = |reason: String| SkippedQuery {
            name: candidate.name.clone(),
            reason,
        };
        let (Some(target), Some(sql), QueryImportStatus::New) =
            (&candidate.target, &plan.sql, &candidate.status)
        else {
            report.skipped.push(skip(skip_reason(&candidate.status)));
            continue;
        };
        match store.save(&target.saved_ref(), sql, false) {
            Ok(_) => report.imported.push(ImportedQuery {
                name: candidate.name.clone(),
                target: target.clone(),
            }),
            Err(error) => report.skipped.push(skip(error.to_string())),
        }
    }
    report
}

fn skip_reason(status: &QueryImportStatus) -> String {
    match status {
        QueryImportStatus::Exists => "已经导入过了".into(),
        QueryImportStatus::Conflict => "目标位置已有同名、内容不同的查询，没有覆盖".into(),
        QueryImportStatus::Unmatched { reason } => reason.clone(),
        QueryImportStatus::New => "无法导入".into(),
    }
}
