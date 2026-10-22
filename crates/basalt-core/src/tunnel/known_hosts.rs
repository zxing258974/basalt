//! 记住 SSH 服务器的主机密钥。文件是 OpenSSH 的 known_hosts 格式,放在 Basalt 的配置目录里,
//! 不读也不改用户自己的 `~/.ssh/known_hosts`。
//!
//! 第一次连接某台服务器时拒绝连接,把指纹交给用户核对,用户确认后才记下;
//! 之后密钥变了一律拒绝,并说清楚记下的和这次的分别是什么、怎么处理。

use std::path::{Path, PathBuf};

use russh::keys::{HashAlg, PublicKey};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// 一台 SSH 服务器出示的主机密钥。只有公钥,可以交给前端显示、再原样交回来确认。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKey {
    pub host: String,
    pub port: u16,
    /// 如 ssh-ed25519、ecdsa-sha2-nistp256、ssh-rsa。
    pub algorithm: String,
    /// `SHA256:` 开头,和 `ssh-keygen -lf` 显示的一样。
    pub fingerprint: String,
    /// OpenSSH 一行公钥的写法(算法 + Base64),确认时交回来记进文件。
    pub public_key: String,
}

impl HostKey {
    pub fn new(host: &str, port: u16, key: &PublicKey) -> Self {
        Self {
            host: normalize_host(host),
            port,
            algorithm: key.algorithm().as_str().to_owned(),
            fingerprint: fingerprint(key),
            public_key: key.to_openssh().unwrap_or_default(),
        }
    }

    pub fn address(&self) -> String {
        address(&self.host, self.port)
    }
}

/// 核对的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyCheck {
    Trusted,
    /// 这台服务器(主机名 + 端口)还没有这种算法的记录。
    Unknown,
    /// 记下的同算法密钥和这次的不一样。`line` 是文件里的行号,`recorded` 是记下的那把的指纹。
    Changed {
        line: usize,
        recorded: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownHosts {
    path: PathBuf,
}

impl KnownHosts {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> CoreResult<HostKeyCheck> {
        let host = normalize_host(host);
        match russh::keys::check_known_hosts_path(&host, port, key, &self.path) {
            Ok(true) => Ok(HostKeyCheck::Trusted),
            Ok(false) => Ok(HostKeyCheck::Unknown),
            Err(russh::keys::Error::KeyChanged { line }) => {
                let recorded =
                    russh::keys::known_hosts::known_host_keys_path(&host, port, &self.path)
                        .ok()
                        .and_then(|keys| keys.into_iter().find(|(at, _)| *at == line))
                        .map(|(_, key)| fingerprint(&key));
                Ok(HostKeyCheck::Changed { line, recorded })
            }
            Err(error) => Err(self.storage_error(error)),
        }
    }

    /// 用户核对过指纹后记下这把密钥。交回来的指纹和公钥对不上就拒绝:
    /// 记进文件的必须是用户看过的那一把。
    pub fn trust(&self, host_key: &HostKey) -> CoreResult<()> {
        let key = PublicKey::from_openssh(host_key.public_key.trim()).map_err(|_| {
            CoreError::Unsupported("主机密钥的格式不对，没有记下，请重新连接一次再确认".into())
        })?;
        if fingerprint(&key) != host_key.fingerprint {
            return Err(CoreError::Unsupported(
                "主机密钥和确认过的指纹对不上，没有记下，请重新连接一次再确认".into(),
            ));
        }
        let host = normalize_host(&host_key.host);
        if let HostKeyCheck::Changed { .. } = self.check(&host, host_key.port, &key)? {
            // 换了密钥的服务器只能手动删掉旧记录,不能在界面上点一下就覆盖。
            return Err(CoreError::HostKeyChanged(format!(
                "{} 已经记着另一把密钥，没有覆盖。确认服务器换了密钥后，先删掉 {} 里它的那一行",
                address(&host, host_key.port),
                self.path.display()
            )));
        }
        russh::keys::known_hosts::learn_known_hosts_path(&host, host_key.port, &key, &self.path)
            .map_err(|error| self.storage_error(error))
    }

    /// 密钥变了时给用户的说明。
    pub(crate) fn changed_message(
        &self,
        seen: &HostKey,
        line: usize,
        recorded: Option<&str>,
    ) -> String {
        let recorded = recorded.map_or_else(String::new, |r| format!("记下的是 {r}，"));
        format!(
            "SSH 服务器 {} 的主机密钥和上次记下的不一样（{recorded}这次是 {}），已拒绝连接。\
             可能是服务器重装或换了密钥，也可能有人在中间冒充它。\
             确认是服务器自己换的，再删掉 {} 第 {line} 行，重新连接时核对新指纹。",
            seen.address(),
            seen.fingerprint,
            self.path.display(),
        )
    }

    fn storage_error(&self, error: impl std::fmt::Display) -> CoreError {
        CoreError::Storage(format!("{}：{error}", self.path.display()))
    }
}

/// OpenSSH 记主机名时不分大小写。
fn normalize_host(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

fn address(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

#[cfg(test)]
#[path = "known_hosts_tests.rs"]
mod tests;
