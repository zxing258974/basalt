//! 读过一次就记在内存里的密码存储。
//!
//! macOS 上每读一次钥匙串都可能弹授权框:包是临时签名,每重打一次系统就当成新程序,
//! 用户只点「允许」(不是「始终允许」)的话,下次读还会再问。SQL 助手每发一句都要用 API Key,
//! 不缓存就句句弹框。这里让同一条在一次运行里只读一次;写和删同时更新内存,不会读到旧值。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use super::{QuietDelete, SecretStore};
use crate::error::{CoreError, CoreResult};

pub struct CachedSecretStore<S> {
    inner: S,
    /// 读到的结果,包括「没有这一条」。读失败(比如用户点了「拒绝」)不记,下次还会再问。
    cache: Mutex<HashMap<String, Option<String>>>,
}

impl<S: SecretStore> CachedSecretStore<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> CoreResult<MutexGuard<'_, HashMap<String, Option<String>>>> {
        self.cache
            .lock()
            .map_err(|_| CoreError::Secret("密码缓存的锁已损坏".into()))
    }
}

impl<S: SecretStore> SecretStore for CachedSecretStore<S> {
    /// 读的时候一直拿着锁:两个地方同时要同一条,也只弹一次框。
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        let mut cache = self.lock()?;
        if let Some(secret) = cache.get(key) {
            return Ok(secret.clone());
        }
        let secret = self.inner.get(key)?;
        cache.insert(key.to_owned(), secret.clone());
        Ok(secret)
    }

    fn set(&self, key: &str, secret: &str) -> CoreResult<()> {
        let mut cache = self.lock()?;
        // 写失败时钥匙串里是什么说不准,忘掉它,下次重新读。
        cache.remove(key);
        self.inner.set(key, secret)?;
        cache.insert(key.to_owned(), Some(secret.to_owned()));
        Ok(())
    }

    fn delete(&self, key: &str) -> CoreResult<()> {
        let mut cache = self.lock()?;
        cache.remove(key);
        self.inner.delete(key)?;
        cache.insert(key.to_owned(), None);
        Ok(())
    }

    fn delete_quietly(&self, key: &str) -> CoreResult<QuietDelete> {
        let mut cache = self.lock()?;
        cache.remove(key);
        let outcome = self.inner.delete_quietly(key)?;
        // 要授权才能删的条目还留在钥匙串里:不记,真要用时照实去读。
        if outcome == QuietDelete::Removed {
            cache.insert(key.to_owned(), None);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use crate::secret::MemorySecretStore;

    /// 数一数真的去读了几次;可以让读、写失败,也可以假装删除要授权。
    #[derive(Default)]
    struct Counting {
        inner: MemorySecretStore,
        reads: AtomicUsize,
        refuse_read: AtomicBool,
        refuse_write: AtomicBool,
        locked_delete: AtomicBool,
    }

    impl SecretStore for Counting {
        fn get(&self, key: &str) -> CoreResult<Option<String>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.refuse_read.load(Ordering::SeqCst) {
                return Err(CoreError::Secret("用户拒绝".into()));
            }
            self.inner.get(key)
        }
        fn set(&self, key: &str, secret: &str) -> CoreResult<()> {
            if self.refuse_write.load(Ordering::SeqCst) {
                return Err(CoreError::Secret("写不进去".into()));
            }
            self.inner.set(key, secret)
        }
        fn delete(&self, key: &str) -> CoreResult<()> {
            self.inner.delete(key)
        }
        fn delete_quietly(&self, key: &str) -> CoreResult<QuietDelete> {
            if self.locked_delete.load(Ordering::SeqCst) {
                return Ok(QuietDelete::NeedsAuthorization);
            }
            self.delete(key).map(|()| QuietDelete::Removed)
        }
    }

    fn reads(store: &CachedSecretStore<Counting>) -> usize {
        store.inner.reads.load(Ordering::SeqCst)
    }

    #[test]
    fn reads_the_keychain_once_per_entry() {
        let store = CachedSecretStore::new(Counting::default());
        store.inner.inner.set("llm:m1:api_key", "sk-test").unwrap();
        for _ in 0..3 {
            assert_eq!(
                store.get("llm:m1:api_key").unwrap().as_deref(),
                Some("sk-test")
            );
        }
        assert_eq!(store.get("missing").unwrap(), None);
        assert_eq!(store.get("missing").unwrap(), None);
        assert_eq!(reads(&store), 2);
    }

    #[test]
    fn writes_and_deletes_keep_the_cache_in_step() {
        let store = CachedSecretStore::new(Counting::default());
        store.set("k", "old").unwrap();
        store.set("k", "new").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("new"));
        store.delete("k").unwrap();
        assert_eq!(store.get("k").unwrap(), None);
        store.set("k", "again").unwrap();
        assert_eq!(store.delete_quietly("k").unwrap(), QuietDelete::Removed);
        assert_eq!(store.get("k").unwrap(), None);
        assert_eq!(reads(&store), 0, "写过、删过的都不用再去读");
    }

    #[test]
    fn a_refused_read_is_asked_again_next_time() {
        let store = CachedSecretStore::new(Counting::default());
        store.inner.inner.set("k", "pw").unwrap();
        store.inner.refuse_read.store(true, Ordering::SeqCst);
        assert!(store.get("k").is_err());
        store.inner.refuse_read.store(false, Ordering::SeqCst);
        assert_eq!(store.get("k").unwrap().as_deref(), Some("pw"));
        assert_eq!(reads(&store), 2);
    }

    #[test]
    fn a_failed_write_forgets_the_old_value() {
        let store = CachedSecretStore::new(Counting::default());
        store.set("k", "old").unwrap();
        store.inner.refuse_write.store(true, Ordering::SeqCst);
        assert!(store.set("k", "new").is_err());
        assert_eq!(store.get("k").unwrap().as_deref(), Some("old"));
        assert_eq!(reads(&store), 1, "写失败后重新去读");
    }

    #[test]
    fn an_entry_left_behind_is_read_for_real() {
        let store = CachedSecretStore::new(Counting::default());
        store.set("k", "pw").unwrap();
        store.inner.locked_delete.store(true, Ordering::SeqCst);
        assert_eq!(
            store.delete_quietly("k").unwrap(),
            QuietDelete::NeedsAuthorization
        );
        assert_eq!(store.get("k").unwrap().as_deref(), Some("pw"));
        assert_eq!(reads(&store), 1);
    }
}
