//! 存在本机文件里的密码:配置目录下的 secrets.json,只有当前用户能读写(0600)。
//!
//! 不用钥匙串:Basalt 是临时签名,每次重装 macOS 都当它是新程序,读钥匙串就要输登录密码放行。
//! 代价是文件里是明文,本账号下运行的程序、备份、同步都能读到;用户知情后选的这个。
//!
//! 以前存在钥匙串里的密码,启动时按配置里用得到的键一次全部搬进来([`FileSecretStore::migrate_legacy`]),
//! 钥匙串的授权框集中在这一次;搬完(或者用户拒绝、取消了授权框)就记下 `legacyDone`,之后再也不碰钥匙串,
//! 没搬过来的由用户重新填。搬家之前用到的,先从钥匙串读那一条。删过、钥匙串里也没有的键记在 `removed` 里,不会再去问钥匙串,
//! 也不会把用户删掉的密码从钥匙串里「复活」。钥匙串里的旧条目不去动:删它们反而要授权。

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::SecretStore;
use crate::error::{CoreError, CoreResult};

/// 文件的内容。不实现 Debug:里面是明文密码。
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
    /// 不用再去钥匙串里找的键:删掉的,或者钥匙串里本来就没有的。
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    removed: BTreeSet<String>,
    /// 钥匙串里的旧密码都搬完了,以后不再去钥匙串找。
    #[serde(default)]
    legacy_done: bool,
}

pub struct FileSecretStore {
    path: PathBuf,
    /// 以前存密码的地方(钥匙串)。只读不写。
    legacy: Option<Box<dyn SecretStore>>,
    state: Mutex<Option<Stored>>,
}

impl FileSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy: None,
            state: Mutex::new(None),
        }
    }

    /// 文件里没有的密码,第一次用到时到这里找一次,找到就搬进文件。
    pub fn with_legacy(mut self, legacy: Box<dyn SecretStore>) -> Self {
        self.legacy = Some(legacy);
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 把 `keys` 里文件中还没有的密码一次从钥匙串读过来,记下「搬完了」,以后不再碰钥匙串。
    /// 钥匙串的授权框都在这一次弹完。有一条读不出来(用户拒绝、取消了授权框)就停下:已经读到的存下,
    /// 剩下的不搬了,也记成搬完——每次启动都弹框正是用户不要的,没搬过来的让用户重新填。
    /// 返回搬过来几条;读不出来时返回那个错误(只用来记日志)。
    pub fn migrate_legacy(&self, keys: &[String]) -> CoreResult<usize> {
        let Some(legacy) = &self.legacy else {
            return Ok(0);
        };
        let mut slot = self.lock()?;
        let mut next = self.load(&mut slot)?;
        if next.legacy_done {
            return Ok(0);
        }
        let mut moved = 0;
        for key in keys {
            if next.secrets.contains_key(key) || next.removed.contains(key) {
                continue;
            }
            match legacy.get(key) {
                Ok(Some(secret)) => {
                    next.secrets.insert(key.clone(), secret);
                    moved += 1;
                }
                Ok(None) => {
                    next.removed.insert(key.clone());
                }
                Err(error) => {
                    next.legacy_done = true;
                    next.removed.clear();
                    self.save(&mut slot, next)?;
                    return Err(error);
                }
            }
        }
        // 搬完了:墓碑只是为了不再去钥匙串找,用不着了。
        next.legacy_done = true;
        next.removed.clear();
        self.save(&mut slot, next)?;
        Ok(moved)
    }

    /// 还要不要去钥匙串找:有旧的来源、还没搬完。
    fn consults_legacy(&self, stored: &Stored) -> bool {
        self.legacy.is_some() && !stored.legacy_done
    }

    fn lock(&self) -> CoreResult<MutexGuard<'_, Option<Stored>>> {
        self.state
            .lock()
            .map_err(|_| CoreError::Secret("密码文件的锁已损坏".into()))
    }

    fn error(&self, error: impl std::fmt::Display) -> CoreError {
        CoreError::Secret(format!("{}：{error}", self.path.display()))
    }

    /// 读进内存。文件不存在当作空;读不懂就报错,绝不覆盖(里面可能有还要找回的密码)。
    fn load(&self, slot: &mut Option<Stored>) -> CoreResult<Stored> {
        if let Some(stored) = slot {
            return Ok(stored.clone());
        }
        let stored = match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| self.error(format!("读不懂，没有改动它（{e}）")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(self.error(e)),
        };
        *slot = Some(stored.clone());
        Ok(stored)
    }

    /// 写同目录的临时文件(一开始就是 0600)再改名:写到一半崩了也不会留下半个文件,也没有一刻是别人可读的。
    fn save(&self, slot: &mut Option<Stored>, next: Stored) -> CoreResult<()> {
        let dir = self.path.parent().ok_or_else(|| self.error("无效的路径"))?;
        std::fs::create_dir_all(dir).map_err(|e| self.error(e))?;
        let text = serde_json::to_string_pretty(&next).map_err(|e| self.error(e))?;
        let tmp = self.path.with_extension("json.tmp");
        let mut file = private_file(&tmp).map_err(|e| self.error(e))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| self.error(e))?;
        drop(file);
        std::fs::rename(&tmp, &self.path).map_err(|e| self.error(e))?;
        *slot = Some(next);
        Ok(())
    }
}

#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // 临时文件可能是上次留下的、权限更宽:一律收紧。
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::create(path)
}

impl SecretStore for FileSecretStore {
    /// 找钥匙串时一直拿着锁:两个地方同时要同一条,也只问一次。
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        let mut slot = self.lock()?;
        let stored = self.load(&mut slot)?;
        if let Some(secret) = stored.secrets.get(key) {
            return Ok(Some(secret.clone()));
        }
        let Some(legacy) = self.legacy.as_ref().filter(|_| !stored.legacy_done) else {
            return Ok(None);
        };
        if stored.removed.contains(key) {
            return Ok(None);
        }
        // 读失败(用户拒绝放行)不记,下次用到时再问。
        let found = legacy.get(key)?;
        let mut next = stored;
        match &found {
            Some(secret) => {
                next.secrets.insert(key.to_owned(), secret.clone());
            }
            None => {
                next.removed.insert(key.to_owned());
            }
        }
        // 搬家失败不影响这次用:下次还会去钥匙串里找。
        if let Err(error) = self.save(&mut slot, next) {
            log::warn!("没能把 {key} 从钥匙串搬进密码文件：{error}");
        }
        Ok(found)
    }

    fn set(&self, key: &str, secret: &str) -> CoreResult<()> {
        let mut slot = self.lock()?;
        let mut next = self.load(&mut slot)?;
        next.secrets.insert(key.to_owned(), secret.to_owned());
        next.removed.remove(key);
        self.save(&mut slot, next)
    }

    fn delete(&self, key: &str) -> CoreResult<()> {
        let mut slot = self.lock()?;
        let stored = self.load(&mut slot)?;
        // 本来就没有、也不用担心钥匙串里那条「复活」的:不必再写一遍文件。
        let tombstone = self.consults_legacy(&stored);
        if !stored.secrets.contains_key(key) && (!tombstone || stored.removed.contains(key)) {
            return Ok(());
        }
        let mut next = stored;
        next.secrets.remove(key);
        if tombstone {
            next.removed.insert(key.to_owned());
        }
        self.save(&mut slot, next)
    }
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
