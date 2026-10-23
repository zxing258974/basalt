//! 服务器监控的命令:取一份快照、终止一个会话。

use basalt_core::monitor::{KillTarget, ServerSnapshot};

use super::CommandResult;
use crate::state::AppState;

pub async fn server_snapshot(state: &AppState, id: &str) -> CommandResult<ServerSnapshot> {
    Ok(state.sessions.server_snapshot(id).await?)
}

pub async fn kill_process(state: &AppState, id: &str, target: &KillTarget) -> CommandResult<()> {
    Ok(state.sessions.kill_process(id, target).await?)
}
