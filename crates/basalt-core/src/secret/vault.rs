//! 把 Basalt 的所有密码和 API Key 合成钥匙串里的一条(macOS)。
//!
//! Basalt 是临时签名,每重装一次,macOS 都当它是新程序,读旧版存的每一条都要用户输登录密码放行。
//! 一条连接一条钥匙串条目时,重装后用到几个连接就问几次;合成一条后,重装后只问这一条。
//!
//! - 旧版单独存的条目在第一次用到时搬进来,再不弹框地试着删掉旧的(旧版存的删不掉就留着,不再读它)。
//! - 删掉的键记进 `removed`:旧条目还留在钥匙串里时,不会被当成没删。
//! - 还没读过这一条时删除(删连接不该弹框),先把键记进本机文件,下次读进来时再删。文件里只有键名。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::{QuietDelete, SecretStore};
use crate::error::{CoreError, CoreResult};

/// 合并后那一条在钥匙串里的账户名。
pub const VAULT_KEY: &str = "basalt:vault";

/// 钥匙串里那一条的内容。不实现 Debug:里面是明文密码。
#[derive(Clone, Default, Serialize, Deserialize)]
struct Vault {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
    #[serde(default)]
    removed: BTreeSet<String>,
}

impl Vault {
    fn with_secret(&self, key: &str, secret: &str) -> Self {
        let mut next = self.clone();
        next.secrets.insert(key.to_owned(), secret.to_owned());
        next.removed.remove(key);
        next
    }

    fn without(&self, keys: &[String]) -> Self {
        let mut next = self.clone();
        for key in keys {
            next.secrets.remove(key);
            next.removed.insert(key.clone());
        }
        next
    }
}

pub struct VaultSecretStore<S> {
    inner: S,
    /// 还没读进钥匙串那一条时要删的键。
    pending_path: PathBuf,
    vault: Mutex<Option<Vault>>,
}

impl<S: SecretStore> VaultSecretStore<S> {
    pub fn new(inner: S, pending_path: impl Into<PathBuf>) -> Self {
        Self {
            inner,
            pending_path: pending_path.into(),
            vault: Mutex::new(None),
        }
    }

    fn lock(&self) -> CoreResult<MutexGuard<'_, Option<Vault>>> {
        self.vault
            .lock()
            .map_err(|_| CoreError::Secret("密码库的锁已损坏".into()))
    }

    /// 读进钥匙串那一条(可能弹授权框),顺手把待删的键删掉。
    fn load(&self, slot: &mut Option<Vault>) -> CoreResult<Vault> {
        if let Some(vault) = slot {
            return Ok(vault.clone());
        }
        let vault = match self.inner.get(VAULT_KEY)? {
            Some(text) => serde_json::from_str(&text).map_err(|error| {
                CoreError::Secret(format!(
                    "钥匙串里 Basalt 的密码条目读不懂，没有改动它：{error}"
                ))
            })?,
            None => Vault::default(),
        };
        let pending = read_pending(&self.pending_path)?;
        let vault = if pending.is_empty() {
            vault
        } else {
            let next = vault.without(&pending);
            self.save(&next)?;
            remove_pending(&self.pending_path)?;
            next
        };
        *slot = Some(vault.clone());
        Ok(vault)
    }

    fn save(&self, vault: &Vault) -> CoreResult<()> {
        let text = serde_json::to_string(vault)
            .map_err(|error| CoreError::Secret(format!("密码库序列化失败：{error}")))?;
        self.inner.set(VAULT_KEY, &text)
    }

    fn replace(&self, slot: &mut Option<Vault>, next: Vault) -> CoreResult<()> {
        self.save(&next)?;
        *slot = Some(next);
        Ok(())
    }

    /// 旧版单独存的条目:不弹框地删,删不掉(旧版存的)就留着。
    fn forget_legacy(&self, key: &str) -> CoreResult<QuietDelete> {
        self.inner.delete_quietly(key)
    }
}

impl<S: SecretStore> SecretStore for VaultSecretStore<S> {
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        let mut slot = self.lock()?;
        let vault = self.load(&mut slot)?;
        if let Some(secret) = vault.secrets.get(key) {
            return Ok(Some(secret.clone()));
        }
        if vault.removed.contains(key) {
            return Ok(None);
        }
        let Some(secret) = self.inner.get(key)? else {
            return Ok(None);
        };
        // 搬家失败不影响这次用:下次还会从旧条目读。
        match self.replace(&mut slot, vault.with_secret(key, &secret)) {
            Ok(()) => {
                if let Err(error) = self.forget_legacy(key) {
                    log::warn!("旧的钥匙串条目 {key} 没能清理：{error}");
                }
            }
            Err(error) => log::warn!("没能把 {key} 搬进 Basalt 的密码条目：{error}"),
        }
        Ok(Some(secret))
    }

    fn set(&self, key: &str, secret: &str) -> CoreResult<()> {
        let mut slot = self.lock()?;
        let vault = self.load(&mut slot)?;
        self.replace(&mut slot, vault.with_secret(key, secret))?;
        if let Err(error) = self.forget_legacy(key) {
            log::warn!("旧的钥匙串条目 {key} 没能清理：{error}");
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> CoreResult<()> {
        let mut slot = self.lock()?;
        let vault = self.load(&mut slot)?;
        self.replace(&mut slot, vault.without(&[key.to_owned()]))?;
        self.forget_legacy(key).map(|_| ())
    }

    /// 绝不弹框:这一条还没读进来就先记下待删,下次读进来时再删。
    fn delete_quietly(&self, key: &str) -> CoreResult<QuietDelete> {
        let mut slot = self.lock()?;
        match slot.as_ref() {
            Some(vault) => {
                let next = vault.without(&[key.to_owned()]);
                self.replace(&mut slot, next)?;
            }
            None => add_pending(&self.pending_path, key)?,
        }
        self.forget_legacy(key)
    }
}

fn pending_error(path: &Path, error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("{}：{error}", path.display()))
}

fn read_pending(path: &Path) -> CoreResult<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| pending_error(path, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(pending_error(path, error)),
    }
}

fn add_pending(path: &Path, key: &str) -> CoreResult<()> {
    let mut keys = read_pending(path)?;
    if !keys.iter().any(|k| k == key) {
        keys.push(key.to_owned());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| pending_error(path, error))?;
    }
    let text = serde_json::to_string(&keys).map_err(|error| pending_error(path, error))?;
    std::fs::write(path, text).map_err(|error| pending_error(path, error))
}

fn remove_pending(path: &Path) -> CoreResult<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(pending_error(path, error))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "vault_tests.rs"]
mod tests;
