//! 记住 SSH 服务器的主机密钥。文件是 OpenSSH 的 known_hosts 格式,放在 Basalt 的配置目录里,
//! 不读也不改用户自己的 `~/.ssh/known_hosts`。
//!
//! 第一次连接某台服务器时拒绝连接,把指纹交给用户核对,用户确认后才记下;
//! 之后密钥变了一律拒绝,并说清楚记下的和这次的分别是什么、怎么处理。
//!
//! - 只记服务器真正出示过、还在等确认的那把([`KnownHosts::expect`]):前端交回来的必须一模一样,
//!   网页里就算被注入了脚本,也没法给随便哪台主机钉上它自己的密钥。
//! - 记过别的类型的密钥、这次却拿出另一种:和密钥变了一样拒绝(中间人可以只拿出服务器没有的那种)。
//! - 文件在但读不了、解析不了:拒绝,不当成空的(当成空的就会弹「第一次连接」的框)。
//! - 写入在一把锁里先查再写,写临时文件再改名,写到一半崩了也不会留下半行。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use russh::keys::{Algorithm, HashAlg, PublicKey};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// 同一个进程里写 known_hosts 的只能有一个:检查和追加之间不能被别的写插进来。
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn write_lock() -> CoreResult<std::sync::MutexGuard<'static, ()>> {
    WRITE_LOCK
        .lock()
        .map_err(|_| CoreError::Storage("known_hosts 的写锁已损坏".into()))
}
/// 等用户确认的主机密钥最多留几把,多了丢最早的。
const MAX_PENDING: usize = 16;

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

    /// 是不是同一台服务器出示的同一把钥匙。
    fn same_as(&self, other: &HostKey) -> bool {
        normalize_host(&self.host) == normalize_host(&other.host)
            && self.port == other.port
            && self.public_key.trim() == other.public_key.trim()
    }
}

/// known_hosts 里这台服务器的一条记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedKey {
    /// 文件里的行号(从 1 起)。
    pub line: usize,
    pub algorithm: String,
    pub fingerprint: String,
}

/// 核对的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyCheck {
    Trusted,
    /// 这台服务器(主机名 + 端口)还没有任何记录。
    Unknown,
    /// 记下的同类型密钥和这次的不一样。`recorded` 是记下的那把的指纹。
    Changed {
        line: usize,
        recorded: Option<String>,
    },
    /// 这台服务器只记过别的类型的密钥,这次拿出来的类型没记过。
    OtherType {
        recorded: Vec<RecordedKey>,
    },
}

/// `pending` 是服务器出示过、还在等用户确认的主机密钥,所有克隆共用一份。
#[derive(Clone, Debug)]
pub struct KnownHosts {
    path: PathBuf,
    pending: Arc<Mutex<Vec<HostKey>>>,
}

/// 只看是不是同一个文件。
impl PartialEq for KnownHosts {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for KnownHosts {}

impl KnownHosts {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            pending: Arc::default(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 这台服务器记下的所有密钥。文件不存在算没有记录;在但读不了、解析不了就报错。
    fn recorded(&self, host: &str, port: u16) -> CoreResult<Vec<(usize, PublicKey)>> {
        match std::fs::File::open(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.unreadable(error)),
        }
        russh::keys::known_hosts::known_host_keys_path(host, port, &self.path)
            .map_err(|error| self.unreadable(error))
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> CoreResult<HostKeyCheck> {
        let host = normalize_host(host);
        let recorded = self.recorded(&host, port)?;
        let same_type: Vec<_> = recorded
            .iter()
            .filter(|(_, k)| k.algorithm() == key.algorithm())
            .collect();
        if same_type
            .iter()
            .any(|(_, k)| k.key_data() == key.key_data())
        {
            return Ok(HostKeyCheck::Trusted);
        }
        if let Some((line, old)) = same_type.first() {
            return Ok(HostKeyCheck::Changed {
                line: *line,
                recorded: Some(fingerprint(old)),
            });
        }
        if recorded.is_empty() {
            return Ok(HostKeyCheck::Unknown);
        }
        Ok(HostKeyCheck::OtherType {
            recorded: recorded
                .iter()
                .map(|(line, k)| RecordedKey {
                    line: *line,
                    algorithm: k.algorithm().as_str().to_owned(),
                    fingerprint: fingerprint(k),
                })
                .collect(),
        })
    }

    /// 记下的密钥类型,连接时排在前面:服务器有好几种主机密钥时,先拿出我们认得的那种。
    pub(crate) fn recorded_algorithms(&self, host: &str, port: u16) -> Vec<Algorithm> {
        self.recorded(&normalize_host(host), port)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, key)| key.algorithm())
            .collect()
    }

    /// 服务器出示了一把没见过的密钥:先记在内存里,等用户确认。
    pub(crate) fn expect(&self, seen: &HostKey) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        pending.retain(|key| !(key.host == seen.host && key.port == seen.port));
        pending.push(seen.clone());
        let overflow = pending.len().saturating_sub(MAX_PENDING);
        pending.drain(..overflow);
    }

    /// 取走等确认的那一把;交回来的和服务器出示的不完全一样就拒绝。
    fn take_pending(&self, host_key: &HostKey) -> CoreResult<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| CoreError::Storage("主机密钥列表的锁已损坏".into()))?;
        let Some(at) = pending.iter().position(|key| key.same_as(host_key)) else {
            return Err(CoreError::Unsupported(
                "这把主机密钥不是刚才服务器出示的那一把，没有记下，请重新连接一次再核对".into(),
            ));
        };
        pending.remove(at);
        Ok(())
    }

    /// 用户核对过指纹后记下这把密钥。只认服务器刚出示过、还在等确认的那一把;已经记着的直接算成功
    /// (同一台服务器同时开两个连接,两边都会来确认)。
    pub fn trust(&self, host_key: &HostKey) -> CoreResult<()> {
        let host = checked_host(&host_key.host)?;
        let key = PublicKey::from_openssh(host_key.public_key.trim()).map_err(|_| {
            CoreError::Unsupported("主机密钥的格式不对，没有记下，请重新连接一次再确认".into())
        })?;
        if fingerprint(&key) != host_key.fingerprint {
            return Err(CoreError::Unsupported(
                "主机密钥和确认过的指纹对不上，没有记下，请重新连接一次再确认".into(),
            ));
        }
        // 检查、取走等确认的那一把、写进文件都在同一把锁里:两个连接一起来确认同一把时,
        // 后到的等先到的写完再看,已经记下了就直接算成功,不会因为等确认的那一把已被取走而报错。
        let _guard = write_lock()?;
        if self.check(&host, host_key.port, &key)? == HostKeyCheck::Trusted {
            return Ok(());
        }
        self.take_pending(host_key)?;
        self.learn_locked(&host, host_key.port, &key)
    }

    /// 记进文件。检查和写入在同一把锁里;写同目录的临时文件再改名,崩在半路也不会留下半行。
    /// 只有测试直接调:界面上的确认走 [`trust`](Self::trust),它自己拿着锁调 `learn_locked`。
    #[cfg(test)]
    pub(crate) fn learn(&self, host: &str, port: u16, key: &PublicKey) -> CoreResult<()> {
        let _guard = write_lock()?;
        self.learn_locked(host, port, key)
    }

    /// 记进文件的主体,调用方已经拿着 [`WRITE_LOCK`]。
    fn learn_locked(&self, host: &str, port: u16, key: &PublicKey) -> CoreResult<()> {
        let host = checked_host(host)?;
        match self.check(&host, port, key)? {
            HostKeyCheck::Trusted => return Ok(()),
            HostKeyCheck::Unknown => {}
            HostKeyCheck::Changed { .. } | HostKeyCheck::OtherType { .. } => {
                // 换了密钥的服务器只能手动删掉旧记录,不能在界面上点一下就覆盖。
                return Err(CoreError::HostKeyChanged(format!(
                    "{} 已经记着另一把密钥，没有覆盖。确认服务器换了密钥后，先删掉 {} 里它的记录",
                    address(&host, port),
                    self.path.display()
                )));
            }
        }
        let mut text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(self.unreadable(error)),
        };
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        let line = key
            .to_openssh()
            .map_err(|error| self.storage_error(error))?;
        text.push_str(&format!("{} {}\n", host_pattern(&host, port), line.trim()));
        self.replace_file(&text)
    }

    fn replace_file(&self, text: &str) -> CoreResult<()> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| self.storage_error("路径不对"))?;
        std::fs::create_dir_all(dir).map_err(|error| self.storage_error(error))?;
        let name = self
            .path
            .file_name()
            .map_or_else(|| "known_hosts".into(), |n| n.to_string_lossy());
        let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let written = (|| {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            restrict_to_owner(&tmp)?;
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(error) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(self.storage_error(error));
        }
        Ok(())
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

    /// 只记过别的类型时的说明:当成密钥变了,不弹「第一次连接」的框。
    pub(crate) fn other_type_message(&self, seen: &HostKey, recorded: &[RecordedKey]) -> String {
        let listed: Vec<_> = recorded
            .iter()
            .map(|r| {
                format!(
                    "第 {} 行 {} {}",
                    r.line,
                    key_type(&r.algorithm),
                    r.fingerprint
                )
            })
            .collect();
        format!(
            "SSH 服务器 {} 这次拿出的是 {} 主机密钥（{}），可是记下的是另一种：{}。已拒绝连接：\
             冒充它的人可以只拿出服务器没有的那种密钥。确认是服务器换了密钥类型，\
             再删掉 {} 里它的这些记录，重新连接时核对新指纹。",
            seen.address(),
            key_type(&seen.algorithm),
            seen.fingerprint,
            listed.join("；"),
            self.path.display(),
        )
    }

    fn unreadable(&self, error: impl std::fmt::Display) -> CoreError {
        CoreError::Storage(format!(
            "读不了记着 SSH 主机密钥的 {}（{error}），为安全起见拒绝连接。请检查这个文件，读不懂的行删掉",
            self.path.display()
        ))
    }

    fn storage_error(&self, error: impl std::fmt::Display) -> CoreError {
        CoreError::Storage(format!("{}：{error}", self.path.display()))
    }
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// OpenSSH 记主机名时不分大小写。
fn normalize_host(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

/// 能写进 known_hosts 的主机名:字母数字和 `. - _ :`(IPv6)、`%`(IPv6 网卡)。
/// `,` 会在同一行里多写一个主机,`|` `@` `*` `?` `!` `[` `]` 是 known_hosts 的语法,一律不收。
pub(crate) fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '%'))
}

fn checked_host(host: &str) -> CoreResult<String> {
    let host = normalize_host(host);
    if valid_host(&host) {
        Ok(host)
    } else {
        Err(CoreError::Unsupported(format!(
            "SSH 主机地址「{host}」里有不能用的字符，没有记下它的主机密钥"
        )))
    }
}

/// known_hosts 里的写法:22 端口只写主机名,别的端口写成 `[host]:port`。
fn host_pattern(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

fn address(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// ssh-ed25519 → ED25519,ecdsa-sha2-nistp256 → ECDSA,ssh-rsa → RSA。
fn key_type(algorithm: &str) -> String {
    if algorithm.starts_with("ecdsa") {
        "ECDSA".into()
    } else if algorithm.contains("rsa") {
        "RSA".into()
    } else {
        algorithm.trim_start_matches("ssh-").to_ascii_uppercase()
    }
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

#[cfg(test)]
#[path = "known_hosts_tests.rs"]
mod tests;
