//! 命令背后的业务逻辑。只依赖 `AppState`,不依赖 Tauri 运行时,方便测试。

use basalt_core::config::{ConnectionConfig, ConnectionDraft};
use basalt_core::driver::{ConnectParams, ServerInfo, TableInfo};
use basalt_core::secret::password_key;
use basalt_core::table::{ApplyOutcome, PageRequest, RowChange, TablePage, TableRef};
use basalt_core::{CoreError, FieldError, session};
use serde::Serialize;

use crate::state::AppState;

mod design;
pub use design::{apply_table_design, preview_table_design, read_table_design};
mod import;
pub use import::{ImportReport, ImportSource, import_connections, scan_import_source};
pub mod query;
mod query_import;
pub use query_import::{import_navicat_queries, scan_navicat_queries};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) fields: Vec<FieldError>,
}

impl From<CoreError> for CommandError {
    fn from(error: CoreError) -> Self {
        log::warn!("命令失败：{error}");
        Self {
            code: error.code(),
            message: error.to_string(),
            fields: error.field_errors().to_vec(),
        }
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    #[serde(flatten)]
    pub(crate) config: ConnectionConfig,
    pub(crate) is_open: bool,
}

pub async fn list_connections(state: &AppState) -> CommandResult<Vec<ConnectionView>> {
    let list = state.load()?;
    let open = state.sessions.open_ids().await;
    Ok(list
        .connections
        .into_iter()
        .map(|config| ConnectionView {
            is_open: open.contains(&config.id),
            config,
        })
        .collect())
}

/// 保存连接时对钥匙串里密码的处理。
#[derive(Debug, PartialEq, Eq)]
enum SecretChange {
    Keep,
    Set(String),
    Delete,
}

fn secret_change(save_password: bool, password: Option<String>) -> SecretChange {
    match (save_password, password) {
        (true, Some(password)) => SecretChange::Set(password),
        (true, None) => SecretChange::Keep,
        (false, _) => SecretChange::Delete,
    }
}

/// 只有影响「怎么连」的改动才需要断开已打开的会话:改名、分组、取消保存密码都不需要。
fn needs_reconnect(
    previous: Option<&ConnectionConfig>,
    next: &ConnectionConfig,
    secret: &SecretChange,
) -> bool {
    previous.is_some_and(|prev| {
        prev.driver != next.driver
            || prev.endpoint != next.endpoint
            || matches!(secret, SecretChange::Set(_))
    })
}

/// 把钥匙串恢复成改动前的值。恢复失败只能记日志:原操作的错误更重要。
fn restore_secret(state: &AppState, key: &str, previous: Option<&str>) {
    let restored = match previous {
        Some(secret) => state.secrets.set(key, secret),
        None => state.secrets.delete(key),
    };
    if let Err(error) = restored {
        log::error!("回滚钥匙串失败，{key} 可能与配置不一致：{error}");
    }
}

/// 写钥匙串在前、写配置在后:钥匙串被拒绝时配置原样不动;配置写失败时把钥匙串回滚。
pub async fn save_connection(
    state: &AppState,
    draft: ConnectionDraft,
) -> CommandResult<ConnectionConfig> {
    let existing_id = draft.id.clone();
    let id = existing_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let change = secret_change(draft.save_password, draft.password.clone());
    let config = draft.into_config(id.clone())?;
    let key = password_key(&id);

    let store = state.store().await;
    let list = store.load()?;
    let previous = match &existing_id {
        Some(existing) => Some(
            list.get(existing)
                .cloned()
                .ok_or_else(|| CoreError::ConnectionNotFound(existing.clone()))?,
        ),
        None => None,
    };

    let old_secret = match change {
        SecretChange::Keep => None,
        _ => state.secrets.get(&key)?,
    };
    // 本来就没存密码时不去碰钥匙串,不需要密码的连接(如 SQLite)不会触发钥匙串授权。
    let change = match change {
        SecretChange::Delete if old_secret.is_none() => SecretChange::Keep,
        other => other,
    };
    match &change {
        SecretChange::Keep => {}
        SecretChange::Set(password) => state.secrets.set(&key, password)?,
        SecretChange::Delete => state.secrets.delete(&key)?,
    }
    if let Err(error) = store.save(&list.upserted(config.clone())) {
        if change != SecretChange::Keep {
            restore_secret(state, &key, old_secret.as_deref());
        }
        return Err(error.into());
    }
    drop(store);

    if needs_reconnect(previous.as_ref(), &config, &change) {
        state.sessions.close(&id).await;
    }
    Ok(config)
}

/// 密码框里勾了「记住」:连接已经成功后再调用,只写钥匙串和 savePassword,不碰会话。
pub async fn remember_password(state: &AppState, id: &str, password: String) -> CommandResult<()> {
    let store = state.store().await;
    let list = store.load()?;
    let config = list
        .get(id)
        .cloned()
        .ok_or_else(|| CoreError::ConnectionNotFound(id.to_owned()))?;
    let key = password_key(id);
    let old_secret = state.secrets.get(&key)?;
    state.secrets.set(&key, &password)?;
    if config.save_password {
        return Ok(());
    }
    let next = ConnectionConfig {
        save_password: true,
        ..config
    };
    if let Err(error) = store.save(&list.upserted(next)) {
        restore_secret(state, &key, old_secret.as_deref());
        return Err(error.into());
    }
    Ok(())
}

/// 配置删掉就算成功;之后清理钥匙串失败只留警告,不让用户看到「删除失败」却已经删了。
pub async fn delete_connection(state: &AppState, id: &str) -> CommandResult<()> {
    {
        let store = state.store().await;
        let list = store.load()?;
        if list.get(id).is_none() {
            return Err(CoreError::ConnectionNotFound(id.to_owned()).into());
        }
        store.save(&list.removed(id))?;
        if let Err(error) = state.secrets.delete(&password_key(id)) {
            log::warn!("连接 {id} 已删除，但清理钥匙串失败：{error}");
        }
    }
    state.sessions.close(id).await;
    Ok(())
}

pub async fn duplicate_connection(state: &AppState, id: &str) -> CommandResult<ConnectionConfig> {
    let new_id = uuid::Uuid::new_v4().to_string();
    let store = state.store().await;
    let list = store.load()?;
    let (next, copy) = list
        .duplicated(id, new_id.clone())
        .ok_or_else(|| CoreError::ConnectionNotFound(id.to_owned()))?;
    let new_key = password_key(&new_id);
    let copied_secret = match state.saved_password(id)? {
        Some(password) => {
            state.secrets.set(&new_key, &password)?;
            true
        }
        None => false,
    };
    if let Err(error) = store.save(&next) {
        if copied_secret {
            restore_secret(state, &new_key, None);
        }
        return Err(error.into());
    }
    Ok(copy)
}

/// 测试表单里的连接。表单没填密码且是已保存的连接时,沿用钥匙串里的密码。
pub async fn test_connection(
    state: &AppState,
    draft: ConnectionDraft,
) -> CommandResult<ServerInfo> {
    let saved = match (&draft.id, &draft.password) {
        (Some(id), None) => state.saved_password(id)?,
        _ => None,
    };
    let password = draft.password.clone().or(saved);
    let config = draft.into_config(String::new())?;
    let params = ConnectParams::from_config(&config, password);
    Ok(session::test_connection(&params).await?)
}

/// 打开连接。`password` 只在未保存密码时由前端询问后传入,不会落盘。
pub async fn open_connection(
    state: &AppState,
    id: &str,
    password: Option<String>,
) -> CommandResult<ServerInfo> {
    let config = state.find(id)?;
    let password = match password {
        Some(p) => Some(p),
        None => state.saved_password(id)?,
    };
    let params = ConnectParams::from_config(&config, password);
    let session = state.sessions.open(id, &params).await?;
    Ok(session.server().clone())
}

pub async fn close_connection(state: &AppState, id: &str) {
    state.sessions.close(id).await;
}

pub async fn list_databases(state: &AppState, id: &str) -> CommandResult<Vec<String>> {
    let session = state.sessions.get(id).await?;
    Ok(session.list_databases().await?)
}

pub async fn list_schemas(
    state: &AppState,
    id: &str,
    database: &str,
) -> CommandResult<Vec<String>> {
    let session = state.sessions.get(id).await?;
    Ok(session.list_schemas(database).await?)
}

pub async fn list_tables(
    state: &AppState,
    id: &str,
    database: &str,
    schema: Option<&str>,
) -> CommandResult<Vec<TableInfo>> {
    let session = state.sessions.get(id).await?;
    Ok(session.list_tables(database, schema).await?)
}

pub async fn read_table_page(
    state: &AppState,
    id: &str,
    table: &TableRef,
    request: &PageRequest,
) -> CommandResult<TablePage> {
    let session = state.sessions.get(id).await?;
    Ok(session.read_table(table, request).await?)
}

/// 返回将要执行的语句(参数内联成字面量),前端在提交前给用户确认。
pub async fn preview_table_changes(
    state: &AppState,
    id: &str,
    table: &TableRef,
    changes: &[RowChange],
) -> CommandResult<Vec<String>> {
    let session = state.sessions.get(id).await?;
    Ok(session.preview_changes(table, changes).await?)
}

/// 在一个事务里提交;任何一条失败都整体回滚,前端保留未提交的修改。
/// `expected` 是用户确认过的预览,重新生成的语句不一样就拒绝执行。
pub async fn apply_table_changes(
    state: &AppState,
    id: &str,
    table: &TableRef,
    changes: &[RowChange],
    expected: &[String],
) -> CommandResult<ApplyOutcome> {
    let session = state.sessions.get(id).await?;
    Ok(session
        .apply_changes(table, changes, Some(expected))
        .await?)
}

#[cfg(test)]
mod tests {
    use basalt_core::config::{DriverKind, Endpoint};

    use super::*;

    fn config(host: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: "c1".into(),
            name: "名".into(),
            group: None,
            driver: DriverKind::Mysql,
            endpoint: Endpoint::Network {
                host: host.into(),
                port: 3306,
                username: "root".into(),
                database: None,
            },
            save_password: true,
        }
    }

    #[test]
    fn secret_change_follows_save_flag_and_supplied_password() {
        assert_eq!(
            secret_change(true, Some("p".into())),
            SecretChange::Set("p".into())
        );
        assert_eq!(secret_change(true, None), SecretChange::Keep);
        assert_eq!(secret_change(false, Some("p".into())), SecretChange::Delete);
    }

    #[test]
    fn reconnect_only_when_how_to_connect_changes() {
        let before = config("db");
        let renamed = ConnectionConfig {
            name: "新名".into(),
            group: Some("组".into()),
            save_password: false,
            ..before.clone()
        };
        assert!(!needs_reconnect(
            Some(&before),
            &renamed,
            &SecretChange::Delete
        ));
        assert!(needs_reconnect(
            Some(&before),
            &config("other"),
            &SecretChange::Keep
        ));
        assert!(needs_reconnect(
            Some(&before),
            &before,
            &SecretChange::Set("p".into())
        ));
        assert!(!needs_reconnect(
            None,
            &before,
            &SecretChange::Set("p".into())
        ));
    }
}
