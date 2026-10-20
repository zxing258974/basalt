//! 导入连接:扫描来源生成预览,再按勾选项写入钥匙串与配置。
//! 扫描与导入各自重新解析来源,明文密码只在导入这一次调用里经过内存,不回传前端。

use std::collections::{HashMap, HashSet};

use basalt_core::config::{ConnectionConfig, DriverKind};
use basalt_core::import::navicat::{self, NavicatLocation};
use basalt_core::import::{
    CandidateStatus, ImportCandidate, SourceConnection, SourcePassword, SourceTarget, classify,
};
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

/// 没有新建、只给已有连接补上了密码。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilledConnection {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub(crate) imported: Vec<ImportedConnection>,
    pub(crate) password_filled: Vec<FilledConnection>,
    pub(crate) skipped: Vec<SkippedConnection>,
}

/// 给已有连接补密码:写进它的钥匙串条目,并把它标为「保存密码」。
struct Fill {
    existing_id: String,
    existing_name: String,
    password: String,
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

/// 导入勾选的连接。和保存单个连接一样:先写钥匙串、再写配置,配置写失败就把这次动过的钥匙串条目恢复原样。
/// 「新增 / 补密码 / 已存在」在持锁后对照最新配置重新判定,不沿用扫描时的结果。
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
    let store = state.store().await;
    let list = store.load()?;
    let statuses: HashMap<String, CandidateStatus> = classify(&sources, &list)
        .into_iter()
        .map(|c| (c.id, c.status))
        .collect();
    let mut creates = Vec::new();
    let mut fills = Vec::new();
    for source in sources.iter().filter(|s| wanted.contains(s.id.as_str())) {
        match (statuses.get(&source.id), &source.password) {
            (
                Some(CandidateStatus::FillPassword {
                    existing_id,
                    existing_name,
                }),
                SourcePassword::Plain(password),
            ) => fills.push(Fill {
                existing_id: existing_id.clone(),
                existing_name: existing_name.clone(),
                password: password.clone(),
            }),
            _ => match plan(source) {
                Ok(item) => creates.push(item),
                Err(reason) => report.skipped.push(SkippedConnection {
                    name: source.name.clone(),
                    reason,
                }),
            },
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

    if creates.is_empty() && fills.is_empty() {
        return Ok(report);
    }

    // (钥匙串条目, 写之前的值):失败时按原值恢复。
    let mut written: Vec<(String, Option<String>)> = Vec::new();
    let secrets = creates
        .iter()
        .filter_map(|c| {
            c.password
                .as_ref()
                .map(|p| (c.config.id.as_str(), p.as_str()))
        })
        .chain(
            fills
                .iter()
                .map(|f| (f.existing_id.as_str(), f.password.as_str())),
        );
    for (id, password) in secrets {
        let key = password_key(id);
        let previous = match state.secrets.get(&key) {
            Ok(previous) => previous,
            Err(error) => {
                rollback(state, &written);
                return Err(error.into());
            }
        };
        if let Err(error) = state.secrets.set(&key, password) {
            rollback(state, &written);
            return Err(error.into());
        }
        written.push((key, previous));
    }
    let with_creates = creates
        .iter()
        .fold(list, |list, item| list.upserted(item.config.clone()));
    let next = fills.iter().fold(with_creates, |list, fill| {
        match list.get(&fill.existing_id).cloned() {
            Some(config) => list.upserted(ConnectionConfig {
                save_password: true,
                ..config
            }),
            None => list,
        }
    });
    if let Err(error) = store.save(&next) {
        rollback(state, &written);
        return Err(error.into());
    }

    log::info!(
        "导入了 {} 个连接，补了 {} 个密码，跳过 {} 个",
        creates.len(),
        fills.len(),
        report.skipped.len()
    );
    report.imported = creates
        .into_iter()
        .map(|item| ImportedConnection {
            id: item.config.id,
            name: item.config.name,
            driver: item.config.driver,
            password_saved: item.password.is_some(),
        })
        .collect();
    report.password_filled = fills
        .into_iter()
        .map(|fill| FilledConnection {
            id: fill.existing_id,
            name: fill.existing_name,
        })
        .collect();
    Ok(report)
}

/// 把这次写过的钥匙串条目恢复成写之前的值(没有原值的删掉)。
fn rollback(state: &AppState, written: &[(String, Option<String>)]) {
    for (key, previous) in written {
        restore_secret(state, key, previous.as_deref());
    }
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
