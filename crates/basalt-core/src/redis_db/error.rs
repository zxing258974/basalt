//! Redis 的错误归到和 SQL 驱动同一套分类里:前端按 `authentication` 重问密码,
//! 监控的专用连接按 `Network` 重连。

use redis::{ErrorKind, RedisError};

use crate::error::{CoreError, tls_hint};

impl From<RedisError> for CoreError {
    fn from(error: RedisError) -> Self {
        // Basalt 自己的脚本报的核对失败(见 edit.lua):原样交给 edit 去解释,不依赖 redis 的显示格式。
        if error.code() == Some("BASALT") {
            return Self::Database(format!("BASALT {}", error.detail().unwrap_or_default()));
        }
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
            // 服务器报的错照 Redis 原样写「错误码 说明」(redis 的显示用的是 NoPerm 这样的种类名):
            // 按 NOPERM、NOSCRIPT 判断原因的地方才对得上。
            Self::Database(match (error.code(), error.detail()) {
                (Some(code), Some(detail)) => format!("{code} {detail}"),
                _ => message,
            })
        }
    }
}

/// 连接断了(服务器重启、空闲超时被踢、网络切换):值得换一条新连接再试一次。
pub(super) fn is_broken(error: &CoreError) -> bool {
    matches!(error, CoreError::Network(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_errors_keep_redis_own_code() {
        let parsed = redis::parse_redis_value(
            b"-NOPERM User app has no permissions to run the 'client|list' command\r\n",
        )
        .unwrap();
        let error = parsed.extract_error().unwrap_err();
        let CoreError::Database(message) = CoreError::from(error) else {
            panic!("应该是数据库错误");
        };
        assert_eq!(
            message,
            "NOPERM User app has no permissions to run the 'client|list' command"
        );
    }
}
