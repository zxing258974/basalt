//! 连接密码的存放处。正式环境用 macOS 钥匙串,测试用内存实现。

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{CoreError, CoreResult};

mod cached;
#[cfg(target_os = "macos")]
mod macos;

pub use cached::CachedSecretStore;

/// [`SecretStore::delete_quietly`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietDelete {
    /// 删掉了,或者本来就没有。
    Removed,
    /// 要用户授权才能删,条目原样留着。钥匙串条目只归存它的那份程序所有,
    /// 每次重新打包临时签名都不一样,旧版 Basalt 存的密码新版删不了。
    NeedsAuthorization,
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> CoreResult<Option<String>>;
    fn set(&self, key: &str, secret: &str) -> CoreResult<()>;
    /// 不存在也算成功。
    fn delete(&self, key: &str) -> CoreResult<()>;
    /// 删除但绝不弹授权框。默认就是 [`delete`](Self::delete);会弹框的实现要覆盖它。
    fn delete_quietly(&self, key: &str) -> CoreResult<QuietDelete> {
        self.delete(key).map(|()| QuietDelete::Removed)
    }
}

/// 连接密码在钥匙串里的账户名。
pub fn password_key(connection_id: &str) -> String {
    format!("connection:{connection_id}:password")
}

pub struct KeychainStore {
    service: String,
}

impl KeychainStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> CoreResult<keyring::Entry> {
        keyring::Entry::new(&self.service, key).map_err(secret_error)
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(secret_error(e)),
        }
    }

    fn set(&self, key: &str, secret: &str) -> CoreResult<()> {
        self.entry(key)?.set_password(secret).map_err(secret_error)
    }

    fn delete(&self, key: &str) -> CoreResult<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_error(e)),
        }
    }

    /// keyring 的删除会先把密码读出来,读别的程序存的密码就要弹框;这里只删不读。
    #[cfg(target_os = "macos")]
    fn delete_quietly(&self, key: &str) -> CoreResult<QuietDelete> {
        macos::delete_generic_password(&self.service, key)
    }
}

fn secret_error(error: keyring::Error) -> CoreError {
    CoreError::Secret(error.to_string())
}

#[derive(Default)]
pub struct MemorySecretStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    fn lock(&self) -> CoreResult<std::sync::MutexGuard<'_, HashMap<String, String>>> {
        self.secrets
            .lock()
            .map_err(|_| CoreError::Secret("内存密码表锁已损坏".into()))
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        Ok(self.lock()?.get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> CoreResult<()> {
        self.lock()?.insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> CoreResult<()> {
        self.lock()?.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_set_get_delete() {
        let store = MemorySecretStore::default();
        let key = password_key("c1");
        assert_eq!(store.get(&key).unwrap(), None);
        store.set(&key, "pw").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("pw"));
        store.delete(&key).unwrap();
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);

        store.set(&key, "pw").unwrap();
        assert_eq!(store.delete_quietly(&key).unwrap(), QuietDelete::Removed);
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn password_key_is_scoped_by_connection() {
        assert_eq!(password_key("abc"), "connection:abc:password");
        assert_ne!(password_key("a"), password_key("b"));
    }
}
