use serde::Serialize;

use crate::tunnel::HostKey;

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

/// 存密码的地方叫什么:macOS 是钥匙串,Windows 是凭据管理器。
pub const SECRET_STORE: &str = if cfg!(target_os = "macos") {
    "钥匙串"
} else {
    "Windows 凭据管理器"
};

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
    #[error("{store}操作失败：{0}", store = SECRET_STORE)]
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
    /// 名字不能用作文件名(保存的查询)。
    #[error("{0}")]
    InvalidName(String),
    /// 同名的东西已经有了,要覆盖得明说。
    #[error("{0}")]
    AlreadyExists(String),
    #[error("{0}")]
    NotFound(String),
    /// MySQL 逐条执行 DDL 时中途失败:前面的已经生效、无法回滚。
    #[error("{0}")]
    PartiallyApplied(String),
    /// 导入导出时读写文件失败。
    #[error("读写文件失败：{0}")]
    File(String),
    /// 导入导出时遇到没法如实写出或读入的值(Excel 装不下的长文本、GBK 表示不了的字符……)。
    #[error("{0}")]
    InvalidData(String),
    /// 用户取消了导入导出。
    #[error("已取消")]
    Cancelled,
    /// SQL 助手请求大模型失败(地址不通、密钥不对、模型报错……)。
    #[error("模型请求失败：{0}")]
    Llm(String),
    /// SSH 隧道建不起来,或者 SSH 服务器转发不到数据库。
    #[error("{0}")]
    Ssh(String),
    /// SSH 密码、私钥或私钥口令不对。和数据库的 `Authentication` 分开,前端据此知道该重问哪一个。
    #[error("{0}")]
    SshAuthentication(String),
    /// 缺 SSH 密码或私钥口令:没保存,也没问过。
    #[error("{0}")]
    SshSecretRequired(String),
    /// 第一次连接这台 SSH 服务器,要用户核对指纹后再连。
    #[error("第一次连接 SSH 服务器 {}，请核对它的主机密钥指纹 {}", .0.address(), .0.fingerprint)]
    HostKeyUnknown(Box<HostKey>),
    /// SSH 服务器的主机密钥和记下的不一样,拒绝连接。
    #[error("{0}")]
    HostKeyChanged(String),
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
            Self::InvalidName(_) => "invalid_name",
            Self::AlreadyExists(_) => "already_exists",
            Self::NotFound(_) => "not_found",
            Self::PartiallyApplied(_) => "partially_applied",
            Self::File(_) => "file",
            Self::InvalidData(_) => "invalid_data",
            Self::Cancelled => "cancelled",
            Self::Llm(_) => "llm",
            Self::Ssh(_) => "ssh",
            Self::SshAuthentication(_) => "ssh_authentication",
            Self::SshSecretRequired(_) => "ssh_secret_required",
            Self::HostKeyUnknown(_) => "host_key_unknown",
            Self::HostKeyChanged(_) => "host_key_changed",
        }
    }

    pub fn field_errors(&self) -> &[FieldError] {
        match self {
            Self::InvalidConfig(errors) => errors,
            _ => &[],
        }
    }

    /// 要用户核对的 SSH 主机密钥。
    pub fn host_key(&self) -> Option<&HostKey> {
        match self {
            Self::HostKeyUnknown(key) => Some(key),
            _ => None,
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
            sqlx::Error::Io(io) => Self::Network(tls_hint(&io.to_string())),
            sqlx::Error::Tls(tls) => {
                Self::Network(format!("TLS 握手失败：{}", tls_hint(&tls.to_string())))
            }
            sqlx::Error::PoolTimedOut => Self::Timeout,
            sqlx::Error::Configuration(cfg) => Self::Database(format!("连接参数有误：{cfg}")),
            other => Self::Database(other.to_string()),
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

/// rustls 校验证书失败的原文(`invalid peer certificate: UnknownIssuer`)换成人话,原文附在后面。
fn tls_hint(message: &str) -> String {
    if !message.contains("invalid peer certificate") {
        return message.to_owned();
    }
    let reason = if message.contains("UnknownIssuer") {
        "服务器证书不是由所选的 CA 签发的（没选 CA 证书时只认常见的公共 CA）"
    } else if message.contains("NotValidForName") {
        "服务器证书里的主机名和连接地址对不上"
    } else if message.contains("Expired") {
        "服务器证书已过期"
    } else if message.contains("NotValidYet") {
        "服务器证书还没到生效时间（也可能是本机时间不对）"
    } else {
        "服务器证书没通过校验"
    };
    format!("TLS 证书校验失败：{reason}（{message}）")
}

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
    fn certificate_failures_are_explained() {
        let unknown = tls_hint("invalid peer certificate: UnknownIssuer");
        assert!(
            unknown.starts_with("TLS 证书校验失败：服务器证书不是由所选的 CA 签发的"),
            "{unknown}"
        );
        assert!(unknown.contains("UnknownIssuer"));
        assert!(
            tls_hint("invalid peer certificate: NotValidForNameContext { .. }").contains("主机名")
        );
        assert!(tls_hint("invalid peer certificate: Expired").contains("过期"));
        assert!(tls_hint("invalid peer certificate: NotValidYet").contains("生效"));
        assert!(tls_hint("invalid peer certificate: BadSignature").contains("没通过校验"));
        assert_eq!(tls_hint("connection refused"), "connection refused");
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
