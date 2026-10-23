//! Redis 连接的命令:库列表(键浏览、命令行、监控在后面加)。

use basalt_core::redis_db::RedisKeyspace;

use super::CommandResult;
use crate::state::AppState;

pub async fn keyspace(state: &AppState, id: &str) -> CommandResult<RedisKeyspace> {
    let session = state.sessions.get_redis(id).await?;
    Ok(session.keyspace().await?)
}
