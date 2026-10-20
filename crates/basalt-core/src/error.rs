use serde::Serialize;

/// 表单字段级的校验错误,前端据此在对应输入框下方提示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FieldError {
    pub fn new(field: &str, message: &str) -> Self {
        Self {
            field: field.to_owned(),
            message: message.to_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("连接配置有误")]
    InvalidConfig(Vec<FieldError>),
    #[error("找不到连接：{0}")]
    ConnectionNotFound(String),
    #[error("连接尚未打开：{0}")]
    NotConnected(String),
    #[error("{0}")]
    Database(String),
    #[error("用户名或密码不正确：{0}")]
    Authentication(String),
    #[error("无法连接到服务器：{0}")]
    Network(String),
    #[error("连接超时，请检查地址、端口和网络")]
    Timeout,
    #[error("钥匙串操作失败：{0}")]
    Secret(String),
    #[error("配置文件读写失败：{0}")]
    Storage(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("连接在打开过程中被关闭或改了配置，请重新打开")]
    Superseded,
    #[error("{0}")]
    InvalidFilter(String),
    #[error("{0}")]
    InvalidEdit(String),
    #[error("{0}")]
    ReadOnly(String),
    /// 某条 UPDATE / DELETE 没有恰好命中一行,整个事务已回滚。
    #[error("{0}")]
    RowMismatch(String),
    /// MySQL 逐条执行 DDL 时中途失败:前面的已经生效、无法回滚。
    #[error("{0}")]
    PartiallyApplied(String),
}

impl CoreError {
    /// 给前端分支判断用的稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidConfig(_) => "invalid_config",
            Self::ConnectionNotFound(_) => "connection_not_found",
            Self::NotConnected(_) => "not_connected",
            Self::Database(_) => "database",
            Self::Authentication(_) => "authentication",
            Self::Network(_) => "network",
            Self::Timeout => "timeout",
            Self::Secret(_) => "secret",
            Self::Storage(_) => "storage",
            Self::Unsupported(_) => "unsupported",
            Self::Superseded => "superseded",
            Self::InvalidFilter(_) => "invalid_filter",
            Self::InvalidEdit(_) => "invalid_edit",
            Self::ReadOnly(_) => "read_only",
            Self::RowMismatch(_) => "row_mismatch",
            Self::PartiallyApplied(_) => "partially_applied",
        }
    }

    pub fn field_errors(&self) -> &[FieldError] {
        match self {
            Self::InvalidConfig(errors) => errors,
            _ => &[],
        }
    }
}

/// MySQL 的 ER_ACCESS_DENIED_ERROR。
const MYSQL_ACCESS_DENIED: u16 = 1045;
/// PostgreSQL 的 invalid_password。
const PG_INVALID_PASSWORD: &str = "28P01";

/// 是否「密码不对」:只有这种情况重新问密码才有意义。
/// MySQL 看错误号,它的 SQLSTATE 28000 还涵盖别的拒绝;PostgreSQL 的 28000
/// 包括 pg_hba 拒绝和角色不存在,所以只认 28P01。
pub(crate) fn is_auth_failure(sqlstate: Option<&str>, mysql_number: Option<u16>) -> bool {
    match mysql_number {
        Some(number) => number == MYSQL_ACCESS_DENIED,
        None => sqlstate == Some(PG_INVALID_PASSWORD),
    }
}

impl From<sqlx::Error> for CoreError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::Database(db) => {
                let message = db.message().to_owned();
                let mysql_number = db
                    .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                    .map(|e| e.number());
                if is_auth_failure(db.code().as_deref(), mysql_number) {
                    Self::Authentication(message)
                } else {
                    Self::Database(message)
                }
            }
            sqlx::Error::Io(io) => Self::Network(io.to_string()),
            sqlx::Error::Tls(tls) => Self::Network(format!("TLS 握手失败：{tls}")),
            sqlx::Error::PoolTimedOut => Self::Timeout,
            sqlx::Error::Configuration(cfg) => Self::Database(format!("连接参数有误：{cfg}")),
            other => Self::Database(other.to_string()),
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_access_denied_is_auth_failure_by_number_not_sqlstate() {
        assert!(is_auth_failure(Some("28000"), Some(1045)));
        // 1130 Host is not allowed 的 SQLSTATE 是 HY000,但即便是 28000 也不算密码错误。
        assert!(!is_auth_failure(Some("28000"), Some(1130)));
    }

    #[test]
    fn postgres_only_invalid_password_is_auth_failure() {
        assert!(is_auth_failure(Some("28P01"), None));
        assert!(
            !is_auth_failure(Some("28000"), None),
            "pg_hba 拒绝、角色不存在不该反复弹密码框"
        );
        assert!(!is_auth_failure(None, None));
    }
}
