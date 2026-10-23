//! Redis 连接的命令:库列表、键浏览与编辑、命令行(监控在后面加)。

use basalt_core::redis_db::RedisKeyspace;
use basalt_core::redis_db::console::ConsoleOutcome;
use basalt_core::redis_db::edit::NewKey;
use basalt_core::redis_db::keys::{KeyDetail, KeyName, ScanPage, ScanRequest};
use basalt_core::table::{ApplyOutcome, RowChange};

use super::CommandResult;
use crate::state::AppState;

pub async fn keyspace(state: &AppState, id: &str) -> CommandResult<RedisKeyspace> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.keyspace().await?)
}

pub async fn scan_keys(
    state: &AppState,
    id: &str,
    db: u32,
    request: &ScanRequest,
) -> CommandResult<ScanPage> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.scan_keys(db, request).await?)
}

pub async fn read_key(state: &AppState, id: &str, db: u32, key: &str) -> CommandResult<KeyDetail> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.read_key(db, key).await?)
}

pub async fn preview_key_changes(
    state: &AppState,
    id: &str,
    key: &str,
    kind: &str,
    changes: &[RowChange],
) -> CommandResult<Vec<String>> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.preview_key_changes(key, kind, changes)?)
}

pub async fn apply_key_changes(
    state: &AppState,
    id: &str,
    db: u32,
    key: &str,
    kind: &str,
    changes: &[RowChange],
    expected: Option<&[String]>,
) -> CommandResult<ApplyOutcome> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session
        .apply_key_changes(db, key, kind, changes, expected)
        .await?)
}

pub async fn set_string(
    state: &AppState,
    id: &str,
    db: u32,
    key: &str,
    original: &str,
    value: &str,
) -> CommandResult<()> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.set_string(db, key, original, value).await?)
}

pub async fn create_key(
    state: &AppState,
    id: &str,
    db: u32,
    new: &NewKey,
) -> CommandResult<KeyName> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.create_key(db, new).await?)
}

pub async fn rename_key(
    state: &AppState,
    id: &str,
    db: u32,
    key: &str,
    name: &str,
) -> CommandResult<KeyName> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.rename_key(db, key, name).await?)
}

pub async fn delete_keys(
    state: &AppState,
    id: &str,
    db: u32,
    keys: &[String],
) -> CommandResult<u64> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.delete_keys(db, keys).await?)
}

pub async fn set_ttl(
    state: &AppState,
    id: &str,
    db: u32,
    key: &str,
    seconds: Option<u64>,
) -> CommandResult<()> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.set_ttl(db, key, seconds).await?)
}

pub async fn run_console(
    state: &AppState,
    id: &str,
    console: &str,
    db: u32,
    line: &str,
    confirmed: bool,
) -> CommandResult<ConsoleOutcome> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.run_console(console, db, line, confirmed).await?)
}

/// 停下、关掉命令行:连接已经关了就没什么可做的。
pub async fn cancel_console(state: &AppState, id: &str, console: &str) {
    if let Ok(session) = state.sessions.get_redis(id).await {
        session.cancel_console(console).await;
    }
}

pub async fn close_console(state: &AppState, id: &str, console: &str) {
    if let Ok(session) = state.sessions.get_redis(id).await {
        session.close_console(console).await;
    }
}

pub async fn command_names(state: &AppState, id: &str) -> CommandResult<Vec<String>> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.command_names().await?)
}
