//! Redis 的错误归到和 SQL 驱动同一套分类里:前端按 `authentication` 重问密码,
//! 监控的专用连接按 `Network` 重连。

use redis::{ErrorKind, RedisError};

use crate::error::{CoreError, tls_hint};

impl From<RedisError> for CoreError {
    fn from(error: RedisError) -> Self {
        let message = error.to_string();
        let auth = error.kind() == ErrorKind::AuthenticationFailed
            || matches!(error.code(), Some("NOAUTH" | "WRONGPASS"));
        if auth {
            Self::Authentication(message)
        } else if error.is_timeout() {
            Self::Timeout
        } else if error.is_io_error()
            || error.is_connection_refusal()
            || error.is_connection_dropped()
        {
            Self::Network(tls_hint(&message))
        } else {
            Self::Database(message)
        }
    }
}

/// 连接断了(服务器重启、空闲超时被踢、网络切换):值得换一条新连接再试一次。
pub(super) fn is_broken(error: &CoreError) -> bool {
    matches!(error, CoreError::Network(_))
}
