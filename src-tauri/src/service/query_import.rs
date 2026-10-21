//! 导入 Navicat 保存的查询:扫描本机 Navicat 的查询文件,对照 Basalt 的连接给出预览,
//! 再把勾选的写进保存的查询。扫描和导入各自重新读一遍来源,不沿用扫描时的结果。

use basalt_core::import::SourceConnection;
use basalt_core::import::navicat::queries::{
    PlannedQuery, QueryImportCandidate, QueryImportReport, import_planned, plan_queries,
    scan_queries,
};
use basalt_core::import::navicat::{self, NavicatLocation};

use super::CommandResult;
use crate::state::AppState;

/// conn.plist 只用来按地址兜底匹配:读不到(没装 Navicat 的连接配置、格式不认识)就只按名字匹配。
fn navicat_connections(local: &NavicatLocation) -> Vec<SourceConnection> {
    local.load().unwrap_or_else(|error| {
        log::info!("读不到 Navicat 的连接配置，只按连接名匹配查询：{error}");
        Vec::new()
    })
}

fn plan(state: &AppState, local: Option<&NavicatLocation>) -> CommandResult<Vec<PlannedQuery>> {
    let local = local.ok_or_else(navicat::local_unavailable)?;
    let queries = scan_queries(&local.settings)?;
    let connections = state.load()?.connections;
    Ok(plan_queries(
        &queries,
        &connections,
        &navicat_connections(local),
        &state.saved_queries,
    ))
}

pub async fn scan_navicat_queries(
    state: &AppState,
    local: Option<&NavicatLocation>,
) -> CommandResult<Vec<QueryImportCandidate>> {
    Ok(plan(state, local)?
        .into_iter()
        .map(|planned| planned.candidate)
        .collect())
}

/// 只写勾选的「新增」项,已有的文件一律不覆盖。
pub async fn import_navicat_queries(
    state: &AppState,
    ids: &[String],
    local: Option<&NavicatLocation>,
) -> CommandResult<QueryImportReport> {
    let plans = plan(state, local)?;
    let report = import_planned(&plans, ids, &state.saved_queries);
    log::info!(
        "导入了 {} 条 Navicat 查询，跳过 {} 条",
        report.imported.len(),
        report.skipped.len()
    );
    Ok(report)
}

#[cfg(test)]
#[path = "query_import_tests.rs"]
mod tests;
