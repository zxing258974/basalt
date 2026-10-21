//! 命令背后的业务逻辑。只依赖 `AppState`,不依赖 Tauri 运行时,方便测试。

use basalt_core::config::{ConnectionConfig, ConnectionDraft};
use basalt_core::driver::{ConnectParams, ServerInfo, TableInfo};
use basalt_core::secret::{QuietDelete, password_key, ssh_secret_key};
use basalt_core::table::{ApplyOutcome, PageRequest, RowChange, TablePage, TableRef};
use basalt_core::tunnel::HostKey;
use basalt_core::{CoreError, FieldError, session};
use serde::Serialize;

use crate::state::AppState;

mod design;
pub use design::{apply_table_design, preview_table_design, read_table_design};
mod import;
pub use import::{ImportReport, ImportSource, import_connections, scan_import_source};
pub mod llm;
pub mod query;
mod query_import;
mod table_ops;
pub use table_ops::{copy_table, drop_table, table_details};
pub mod transfer;
mod tunnel;
pub use query_import::{import_navicat_queries, scan_navicat_queries};
use tunnel::SecretEdit;
pub use tunnel::{remember_ssh_secret, trust_host_key};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) fields: Vec<FieldError>,
    /// 第一次连接的 SSH 服务器的主机密钥,前端拿给用户核对。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) host_key: Option<Box<HostKey>>,
}

impl From<CoreError> for CommandError {
    fn from(error: CoreError) -> Self {
        log::warn!("命令失败：{error}");
        Self {
            code: error.code(),
            message: error.to_string(),
            fields: error.field_errors().to_vec(),
            host_key: error.host_key().cloned().map(Box::new),
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
/// `secrets` 是数据库密码和 SSH 密码这次的改动,换了新密码也要重连。
fn needs_reconnect(
    previous: Option<&ConnectionConfig>,
    next: &ConnectionConfig,
    secrets: &[&SecretChange],
) -> bool {
    previous.is_some_and(|prev| {
        prev.driver != next.driver
            || prev.endpoint != next.endpoint
            || tunnel::route_changed(prev, next)
            || secrets.iter().any(|s| matches!(s, SecretChange::Set(_)))
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
/// 数据库密码和 SSH 密码各是一条,一起写、一起回滚。
pub async fn save_connection(
    state: &AppState,
    draft: ConnectionDraft,
) -> CommandResult<ConnectionConfig> {
    let existing_id = draft.id.clone();
    let id = existing_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let change = secret_change(draft.save_password, draft.password.clone());
    let ssh_change = tunnel::ssh_secret_change(draft.ssh.as_ref(), draft.ssh_secret.clone());
    let config = draft.into_config(id.clone())?;

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

    // 原来就没存密码(或者是新建的连接)时钥匙串里没有它的密码,不用去读:
    // 不需要密码的连接(如 SQLite)保存时不会触发钥匙串授权。
    let had_password = previous.as_ref().is_some_and(|p| p.save_password);
    let had_ssh_secret = previous
        .as_ref()
        .and_then(|p| p.ssh.as_ref())
        .is_some_and(|t| t.save_secret);
    let edits = [
        SecretEdit::plan(state, password_key(&id), change, had_password)?,
        SecretEdit::plan(state, ssh_secret_key(&id), ssh_change, had_ssh_secret)?,
    ];
    tunnel::apply_all(state, &edits)?;
    if let Err(error) = store.save(&list.upserted(config.clone())) {
        tunnel::revert_all(state, &edits);
        return Err(error.into());
    }
    drop(store);

    let changes: Vec<_> = edits.iter().map(SecretEdit::change).collect();
    if needs_reconnect(previous.as_ref(), &config, &changes) {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedConnection {
    /// 密码没能从钥匙串里删掉,还留在那里。
    pub password_left: bool,
}

/// 配置删掉就算成功。之后清理钥匙串不弹授权框:删连接不该要用户输登录密码,
/// 何况配置已经没了,拒绝授权也挽回不了;删不掉的密码留在钥匙串里,如实告诉前端。
pub async fn delete_connection(state: &AppState, id: &str) -> CommandResult<DeletedConnection> {
    let password_left = {
        let store = state.store().await;
        let list = store.load()?;
        if list.get(id).is_none() {
            return Err(CoreError::ConnectionNotFound(id.to_owned()).into());
        }
        store.save(&list.removed(id))?;
        // 两条都要删,不能用 any() 短路掉第二条。
        let left: Vec<bool> = [password_key(id), ssh_secret_key(id)]
            .iter()
            .map(|key| match state.secrets.delete_quietly(key) {
                Ok(QuietDelete::Removed) => false,
                Ok(QuietDelete::NeedsAuthorization) => {
                    log::warn!("连接 {id} 已删除，{key} 要授权才能删，留在了钥匙串里");
                    true
                }
                Err(error) => {
                    log::warn!("连接 {id} 已删除，但清理钥匙串里的 {key} 失败：{error}");
                    true
                }
            })
            .collect();
        left.contains(&true)
    };
    state.sessions.close(id).await;
    Ok(DeletedConnection { password_left })
}

pub async fn duplicate_connection(state: &AppState, id: &str) -> CommandResult<ConnectionConfig> {
    let new_id = uuid::Uuid::new_v4().to_string();
    let store = state.store().await;
    let list = store.load()?;
    let (next, copy) = list
        .duplicated(id, new_id.clone())
        .ok_or_else(|| CoreError::ConnectionNotFound(id.to_owned()))?;
    let source = list
        .get(id)
        .cloned()
        .ok_or_else(|| CoreError::ConnectionNotFound(id.to_owned()))?;
    let copies = [
        (password_key(&new_id), state.saved_password(&source)?),
        (ssh_secret_key(&new_id), state.saved_ssh_secret(&source)?),
    ];
    let mut copied: Vec<String> = Vec::new();
    for (key, secret) in &copies {
        let Some(secret) = secret else { continue };
        if let Err(error) = state.secrets.set(key, secret) {
            copied
                .iter()
                .for_each(|key| restore_secret(state, key, None));
            return Err(error.into());
        }
        copied.push(key.clone());
    }
    if let Err(error) = store.save(&next) {
        copied
            .iter()
            .for_each(|key| restore_secret(state, key, None));
        return Err(error.into());
    }
    Ok(copy)
}

/// 测试表单里的连接(经隧道时连隧道一起测)。表单没填密码且是已保存的连接时,沿用钥匙串里的密码;
/// SSH 密码同理。
pub async fn test_connection(
    state: &AppState,
    draft: ConnectionDraft,
) -> CommandResult<ServerInfo> {
    let saved = match &draft.id {
        Some(id) if draft.password.is_none() || draft.ssh_secret.is_none() => state.find(id).ok(),
        _ => None,
    };
    let password = match (&draft.password, &saved) {
        (Some(password), _) => Some(password.clone()),
        (None, Some(saved)) => state.saved_password(saved)?,
        (None, None) => None,
    };
    let ssh_secret = match (&draft.ssh_secret, &saved) {
        (Some(secret), _) => Some(secret.clone()),
        (None, Some(saved)) => state.saved_ssh_secret(saved)?,
        (None, None) => None,
    };
    let config = draft.into_config(String::new())?;
    let params = ConnectParams::from_config(&config, password, ssh_secret, &state.known_hosts);
    Ok(session::test_connection(&params).await?)
}

/// 打开连接。`password`、`ssh_secret` 只在没保存时由前端询问后传入,不会落盘。
pub async fn open_connection(
    state: &AppState,
    id: &str,
    password: Option<String>,
    ssh_secret: Option<String>,
) -> CommandResult<ServerInfo> {
    let config = state.find(id)?;
    let password = match password {
        Some(p) => Some(p),
        None => state.saved_password(&config)?,
    };
    let ssh_secret = match ssh_secret {
        Some(s) => Some(s),
        None => state.saved_ssh_secret(&config)?,
    };
    let params = ConnectParams::from_config(&config, password, ssh_secret, &state.known_hosts);
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
    use basalt_core::config::{DriverKind, Endpoint, SslSettings};

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
            ssh: None,
            ssl: SslSettings::default(),
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
            &[&SecretChange::Delete]
        ));
        assert!(needs_reconnect(
            Some(&before),
            &config("other"),
            &[&SecretChange::Keep]
        ));
        assert!(needs_reconnect(
            Some(&before),
            &before,
            &[&SecretChange::Keep, &SecretChange::Set("p".into())]
        ));
        assert!(!needs_reconnect(
            None,
            &before,
            &[&SecretChange::Set("p".into())]
        ));
    }
}
