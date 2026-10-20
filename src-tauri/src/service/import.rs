//! 导入连接:扫描来源生成预览,再按勾选项写入钥匙串与配置。
//! 扫描与导入各自重新解析来源,明文密码只在导入这一次调用里经过内存,不回传前端。

use std::collections::HashSet;

use basalt_core::config::{ConnectionConfig, DriverKind};
use basalt_core::import::navicat::{self, NavicatLocation};
use basalt_core::import::{ImportCandidate, SourceConnection, SourceTarget, classify};
use basalt_core::secret::password_key;
use basalt_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

use super::{CommandResult, restore_secret};
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImportSource {
    LocalNavicat,
    NcxFile { path: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedConnection {
    pub(crate) id: String,
    pub(crate) name: String,
    /// SQLite 这类文件库没有密码,汇报时不必提示输入。
    pub(crate) driver: DriverKind,
    /// 密码已写进钥匙串;否则首次打开时要输入。
    pub(crate) password_saved: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedConnection {
    pub(crate) name: String,
    pub(crate) reason: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub(crate) imported: Vec<ImportedConnection>,
    pub(crate) skipped: Vec<SkippedConnection>,
}

/// `local` 是本机 Navicat 的位置;找不到用户主目录时为 `None`,只影响本机来源。
fn load_source(
    source: &ImportSource,
    local: Option<&NavicatLocation>,
) -> CoreResult<Vec<SourceConnection>> {
    match source {
        ImportSource::LocalNavicat => local
            .ok_or_else(|| CoreError::Storage("找不到用户主目录，无法定位本机 Navicat".into()))?
            .load(),
        ImportSource::NcxFile { path } => navicat::load_ncx(path.as_ref()),
    }
}

/// 扫描来源,对照已有连接给出预览。不写任何东西。
pub async fn scan_import_source(
    state: &AppState,
    source: &ImportSource,
    local: Option<&NavicatLocation>,
) -> CommandResult<Vec<ImportCandidate>> {
    let sources = load_source(source, local)?;
    Ok(classify(&sources, &state.load()?))
}

/// 一个准备写入的连接和它的明文密码。
struct Planned {
    config: ConnectionConfig,
    password: Option<String>,
}

fn plan(source: &SourceConnection) -> Result<Planned, String> {
    let draft = source.to_draft().ok_or_else(|| match &source.target {
        SourceTarget::Unsupported(reason) => reason.clone(),
        SourceTarget::Supported { .. } => "无法导入".to_owned(),
    })?;
    let password = draft.password.clone();
    let config = draft
        .into_config(uuid::Uuid::new_v4().to_string())
        .map_err(|error| {
            let fields: Vec<_> = error
                .field_errors()
                .iter()
                .map(|e| e.message.clone())
                .collect();
            if fields.is_empty() {
                error.to_string()
            } else {
                fields.join("；")
            }
        })?;
    Ok(Planned { config, password })
}

/// 导入勾选的连接。和保存单个连接一样:先写钥匙串、再写配置,配置写失败就把这次写进钥匙串的都撤掉。
/// 单个连接不合格只记进「跳过」,钥匙串或配置文件出错才让整次导入失败。
pub async fn import_connections(
    state: &AppState,
    source: &ImportSource,
    ids: &[String],
    local: Option<&NavicatLocation>,
) -> CommandResult<ImportReport> {
    let sources = load_source(source, local)?;
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut report = ImportReport::default();
    let mut planned = Vec::new();
    for source in sources.iter().filter(|s| wanted.contains(s.id.as_str())) {
        match plan(source) {
            Ok(item) => planned.push(item),
            Err(reason) => report.skipped.push(SkippedConnection {
                name: source.name.clone(),
                reason,
            }),
        }
    }
    let mut missing: HashSet<&str> = sources.iter().map(|s| s.id.as_str()).collect();
    report.skipped.extend(
        ids.iter()
            .filter(|id| missing.insert(id.as_str()))
            .map(|id| SkippedConnection {
                name: id.clone(),
                reason: "扫描之后来源有变化，找不到这个连接，请重新扫描".to_owned(),
            }),
    );
    if planned.is_empty() {
        return Ok(report);
    }

    let store = state.store().await;
    let list = store.load()?;
    let mut written = Vec::new();
    for item in &planned {
        let Some(password) = &item.password else {
            continue;
        };
        let key = password_key(&item.config.id);
        if let Err(error) = state.secrets.set(&key, password) {
            rollback(state, &written);
            return Err(error.into());
        }
        written.push(key);
    }
    let next = planned
        .iter()
        .fold(list, |list, item| list.upserted(item.config.clone()));
    if let Err(error) = store.save(&next) {
        rollback(state, &written);
        return Err(error.into());
    }

    log::info!(
        "导入了 {} 个连接，跳过 {} 个",
        planned.len(),
        report.skipped.len()
    );
    report.imported = planned
        .into_iter()
        .map(|item| ImportedConnection {
            id: item.config.id,
            name: item.config.name,
            driver: item.config.driver,
            password_saved: item.password.is_some(),
        })
        .collect();
    Ok(report)
}

fn rollback(state: &AppState, keys: &[String]) {
    for key in keys {
        restore_secret(state, key, None);
    }
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
