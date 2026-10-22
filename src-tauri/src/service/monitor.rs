//! 服务器监控的命令:取一份快照、终止一个会话。

use basalt_core::monitor::{KillTarget, ServerSnapshot};

use super::CommandResult;
use crate::state::AppState;

pub async fn server_snapshot(state: &AppState, id: &str) -> CommandResult<ServerSnapshot> {
    let session = state.sessions.get(id).await?;
    Ok(session.server_snapshot().await?)
}

pub async fn kill_process(state: &AppState, id: &str, target: &KillTarget) -> CommandResult<()> {
    let session = state.sessions.get(id).await?;
    Ok(session.kill_process(target).await?)
}
