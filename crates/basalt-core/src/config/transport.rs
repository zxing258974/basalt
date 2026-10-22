//! 连接怎么到达数据库:经不经 SSH 隧道、要不要 SSL。密码和私钥口令不在这里,见 [`crate::secret`]。
//!
//! 两项都带 `#[serde(default)]`:M4 以前存的 connections.json 没有它们,读进来是「直连、SSL 优先」,
//! 和那时 sqlx 的默认行为一样。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::FieldError;

pub const DEFAULT_SSH_PORT: u16 = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SshAuthMethod {
    Password,
    PrivateKey,
}

/// 先登录 SSH 服务器,再由它转发到数据库。数据库的主机、端口是从 SSH 服务器那边看的地址。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnel {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuthMethod,
    /// 私钥文件的绝对路径。密码登录时不用。
    #[serde(default)]
    pub private_key_path: Option<String>,
    /// SSH 密码(或私钥口令)存在密钥库里。没存时,密码登录每次打开前要问。
    #[serde(default)]
    pub save_secret: bool,
}

impl SshTunnel {
    /// 打开前一定要问用户的:密码登录又没存密码。私钥没存口令时先不问,私钥真有口令再问。
    pub fn needs_prompt(&self) -> bool {
        self.auth == SshAuthMethod::Password && !self.save_secret
    }

    fn normalized(self) -> Self {
        let private_key_path = match self.auth {
            SshAuthMethod::Password => None,
            SshAuthMethod::PrivateKey => non_blank(self.private_key_path),
        };
        Self {
            host: self.host.trim().to_owned(),
            username: self.username.trim().to_owned(),
            private_key_path,
            ..self
        }
    }
}

/// 和 MySQL 的 ssl-mode 对应;PostgreSQL 的 prefer / require / verify-ca / verify-full 一一对上
/// (allow 当作 preferred)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SslMode {
    Disabled,
    /// 服务器支持就加密,不校验证书。M4 以前的连接都是这样(sqlx 的默认值)。
    #[default]
    Preferred,
    /// 必须加密,不校验证书。
    Required,
    /// 加密并校验证书由可信的 CA 签发,不核对主机名。
    VerifyCa,
    /// 再核对证书里的主机名。
    VerifyIdentity,
}

impl SslMode {
    pub fn verifies_certificate(self) -> bool {
        matches!(self, Self::VerifyCa | Self::VerifyIdentity)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SslSettings {
    #[serde(default)]
    pub mode: SslMode,
    /// 校验服务器证书用的 CA 证书;不填时用内置的公共 CA 列表。
    #[serde(default)]
    pub ca_path: Option<String>,
    /// 客户端证书和私钥,服务器要求证书登录时填,两个要一起填。
    #[serde(default)]
    pub client_cert_path: Option<String>,
    #[serde(default)]
    pub client_key_path: Option<String>,
}

impl SslSettings {
    fn normalized(self) -> Self {
        Self {
            mode: self.mode,
            ca_path: non_blank(self.ca_path),
            client_cert_path: non_blank(self.client_cert_path),
            client_key_path: non_blank(self.client_key_path),
        }
    }

    /// 要用到的证书文件:禁用 SSL 时一个都不用。
    pub fn files(&self) -> Vec<(&'static str, &str)> {
        if self.mode == SslMode::Disabled {
            return Vec::new();
        }
        [
            ("CA 证书", &self.ca_path),
            ("客户端证书", &self.client_cert_path),
            ("客户端私钥", &self.client_key_path),
        ]
        .into_iter()
        .filter_map(|(label, path)| Some((label, path.as_deref()?)))
        .collect()
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|v| expand_home(v.trim()))
        .filter(|v| !v.is_empty())
}

/// 路径开头的 `~` 换成用户主目录(私钥常写成 `~/.ssh/id_ed25519`)。别的写法原样返回。
pub(crate) fn expand_home(path: &str) -> String {
    let rest = match path {
        "~" => "",
        _ => match path.strip_prefix("~/") {
            Some(rest) => rest,
            None => return path.to_owned(),
        },
    };
    match std::env::home_dir() {
        Some(home) if rest.is_empty() => home.display().to_string(),
        Some(home) => home.join(rest).display().to_string(),
        None => path.to_owned(),
    }
}

/// 整理表单里的隧道和 SSL:去首尾空白,用不到的路径清掉。
pub(super) fn normalize(
    ssh: Option<SshTunnel>,
    ssl: SslSettings,
) -> (Option<SshTunnel>, SslSettings) {
    (ssh.map(SshTunnel::normalized), ssl.normalized())
}

fn absolute(path: &str) -> bool {
    Path::new(&expand_home(path.trim())).is_absolute()
}

pub(super) fn validate_ssh(ssh: &SshTunnel, errors: &mut Vec<FieldError>) {
    let host = ssh.host.trim();
    if host.is_empty() {
        errors.push(FieldError::new("sshHost", "请填写 SSH 主机地址"));
    } else if host.chars().any(char::is_whitespace) {
        errors.push(FieldError::new("sshHost", "SSH 主机地址不能包含空格"));
    } else if !crate::tunnel::valid_host(host) {
        // `,` `|` `*` 这些在 known_hosts 里有特别的意思,记主机密钥时会多匹配上别的主机。
        errors.push(FieldError::new(
            "sshHost",
            "SSH 主机地址只能有字母、数字和 . - _ :（IPv6 不用加方括号）",
        ));
    }
    if ssh.port == 0 {
        errors.push(FieldError::new("sshPort", "端口范围是 1–65535"));
    }
    if ssh.username.trim().is_empty() {
        errors.push(FieldError::new("sshUsername", "请填写 SSH 用户名"));
    }
    if ssh.auth == SshAuthMethod::PrivateKey {
        match ssh.private_key_path.as_deref().map(str::trim) {
            None | Some("") => errors.push(FieldError::new("sshPrivateKey", "请选择私钥文件")),
            Some(path) if !absolute(path) => {
                errors.push(FieldError::new("sshPrivateKey", "请使用绝对路径"));
            }
            Some(_) => {}
        }
    }
}

pub(super) fn validate_ssl(ssl: &SslSettings, tunneled: bool, errors: &mut Vec<FieldError>) {
    if ssl.mode == SslMode::Disabled {
        return;
    }
    // 经隧道时驱动连的是本机转发端口,证书里的主机名永远对不上 127.0.0.1。
    if tunneled && ssl.mode == SslMode::VerifyIdentity {
        errors.push(FieldError::new(
            "sslMode",
            "经 SSH 隧道连接时没法核对主机名（实际连的是本机的转发端口），请改用「校验 CA」",
        ));
    }
    let blank = |path: &Option<String>| path.as_deref().is_none_or(|p| p.trim().is_empty());
    for (field, path) in [
        ("sslCa", &ssl.ca_path),
        ("sslCert", &ssl.client_cert_path),
        ("sslKey", &ssl.client_key_path),
    ] {
        if !blank(path) && path.as_deref().is_some_and(|p| !absolute(p)) {
            errors.push(FieldError::new(field, "请使用绝对路径"));
        }
    }
    match (blank(&ssl.client_cert_path), blank(&ssl.client_key_path)) {
        (false, true) => errors.push(FieldError::new("sslKey", "客户端证书和私钥要一起填")),
        (true, false) => errors.push(FieldError::new("sslCert", "客户端证书和私钥要一起填")),
        _ => {}
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
