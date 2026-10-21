use std::collections::HashMap;
use std::sync::Arc;

use super::*;
use crate::secret::MemorySecretStore;

/// 假的钥匙串:记下每个键被读了几次;`locked` 里的旧条目「是旧版存的」,不弹框删不掉。
#[derive(Default)]
struct Keychain {
    items: MemorySecretStore,
    reads: Mutex<HashMap<String, usize>>,
    locked: Mutex<BTreeSet<String>>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Keychain>);

impl Shared {
    fn reads(&self, key: &str) -> usize {
        self.0.reads.lock().unwrap().get(key).copied().unwrap_or(0)
    }
    fn raw(&self, key: &str) -> Option<String> {
        self.0.items.get(key).unwrap()
    }
    fn put_legacy(&self, key: &str, secret: &str, locked: bool) {
        self.0.items.set(key, secret).unwrap();
        if locked {
            self.0.locked.lock().unwrap().insert(key.to_owned());
        }
    }
}

impl SecretStore for Shared {
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        *self
            .0
            .reads
            .lock()
            .unwrap()
            .entry(key.to_owned())
            .or_default() += 1;
        self.0.items.get(key)
    }
    fn set(&self, key: &str, secret: &str) -> CoreResult<()> {
        self.0.items.set(key, secret)
    }
    fn delete(&self, key: &str) -> CoreResult<()> {
        self.0.items.delete(key)
    }
    fn delete_quietly(&self, key: &str) -> CoreResult<QuietDelete> {
        if self.0.locked.lock().unwrap().contains(key) {
            return Ok(QuietDelete::NeedsAuthorization);
        }
        self.delete(key).map(|()| QuietDelete::Removed)
    }
}

struct Setup {
    keychain: Shared,
    dir: tempfile::TempDir,
}

impl Setup {
    fn new() -> Self {
        Self {
            keychain: Shared::default(),
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn pending(&self) -> PathBuf {
        self.dir.path().join("pending-secret-deletes.json")
    }
    /// 一次运行:新开一个密码库,内存里什么都没有,和重新启动 Basalt 一样。
    fn run(&self) -> VaultSecretStore<Shared> {
        VaultSecretStore::new(self.keychain.clone(), self.pending())
    }
}

#[test]
fn everything_lives_in_one_keychain_item() {
    let setup = Setup::new();
    let store = setup.run();
    store.set("connection:a:password", "pa").unwrap();
    store.set("llm:m1:api_key", "sk-1").unwrap();
    assert_eq!(
        setup.keychain.raw("connection:a:password"),
        None,
        "不再单独存"
    );

    let next = setup.run();
    for _ in 0..3 {
        assert_eq!(
            next.get("connection:a:password").unwrap().as_deref(),
            Some("pa")
        );
        assert_eq!(next.get("llm:m1:api_key").unwrap().as_deref(), Some("sk-1"));
    }
    assert_eq!(setup.keychain.reads(VAULT_KEY), 2, "每次运行只读这一条一次");
}

#[test]
fn old_single_items_move_in_when_first_used() {
    let setup = Setup::new();
    setup
        .keychain
        .put_legacy("connection:mine:password", "new-build", false);
    setup
        .keychain
        .put_legacy("connection:old:password", "old-build", true);
    let store = setup.run();
    assert_eq!(
        store.get("connection:mine:password").unwrap().as_deref(),
        Some("new-build")
    );
    assert_eq!(
        store.get("connection:old:password").unwrap().as_deref(),
        Some("old-build")
    );
    assert_eq!(
        setup.keychain.raw("connection:mine:password"),
        None,
        "自己存的旧条目删掉了"
    );
    assert!(
        setup.keychain.raw("connection:old:password").is_some(),
        "旧版存的删不掉,留着"
    );

    let next = setup.run();
    assert_eq!(
        next.get("connection:old:password").unwrap().as_deref(),
        Some("old-build")
    );
    assert_eq!(
        setup.keychain.reads("connection:old:password"),
        1,
        "搬进来以后不再读旧条目"
    );
}

#[test]
fn a_deleted_secret_stays_deleted_even_if_the_old_item_is_left() {
    let setup = Setup::new();
    setup
        .keychain
        .put_legacy("connection:c:password", "pw", true);
    let store = setup.run();
    assert_eq!(
        store.get("connection:c:password").unwrap().as_deref(),
        Some("pw")
    );
    store.delete("connection:c:password").unwrap();
    assert_eq!(store.get("connection:c:password").unwrap(), None);
    assert_eq!(setup.run().get("connection:c:password").unwrap(), None);

    store.set("connection:c:password", "again").unwrap();
    assert_eq!(
        setup.run().get("connection:c:password").unwrap().as_deref(),
        Some("again")
    );
}

#[test]
fn deleting_before_anything_was_read_never_touches_the_vault() {
    let setup = Setup::new();
    setup.run().set("connection:gone:password", "pw").unwrap();
    setup.run().set("llm:m:api_key", "sk").unwrap();

    let quiet = setup.run();
    assert_eq!(
        quiet.delete_quietly("connection:gone:password").unwrap(),
        QuietDelete::Removed
    );
    assert_eq!(setup.keychain.reads(VAULT_KEY), 2, "删除时没去读,不会弹框");
    assert!(setup.pending().exists());

    let later = setup.run();
    assert_eq!(later.get("llm:m:api_key").unwrap().as_deref(), Some("sk"));
    assert_eq!(later.get("connection:gone:password").unwrap(), None);
    assert!(!setup.pending().exists(), "读进来时删掉了,待删清单也清了");
    assert_eq!(setup.run().get("connection:gone:password").unwrap(), None);
}

#[test]
fn quiet_delete_reports_an_old_item_it_could_not_remove() {
    let setup = Setup::new();
    setup
        .keychain
        .put_legacy("connection:x:password", "pw", true);
    let store = setup.run();
    assert_eq!(
        store.delete_quietly("connection:x:password").unwrap(),
        QuietDelete::NeedsAuthorization
    );
    assert_eq!(setup.run().get("connection:x:password").unwrap(), None);

    let loaded = setup.run();
    loaded.set("connection:y:password", "py").unwrap();
    assert_eq!(
        loaded.delete_quietly("connection:y:password").unwrap(),
        QuietDelete::Removed
    );
    assert_eq!(setup.run().get("connection:y:password").unwrap(), None);
}

#[test]
fn a_vault_it_cannot_read_is_left_alone() {
    let setup = Setup::new();
    setup.keychain.put_legacy(VAULT_KEY, "{不是 JSON", false);
    let store = setup.run();
    let error = store.get("connection:a:password").unwrap_err();
    assert!(error.to_string().contains("读不懂"), "{error}");
    assert!(store.set("connection:a:password", "pw").is_err());
    assert_eq!(setup.keychain.raw(VAULT_KEY).as_deref(), Some("{不是 JSON"));
}

#[test]
fn missing_secrets_are_not_stored() {
    let setup = Setup::new();
    let store = setup.run();
    assert_eq!(store.get("connection:none:password").unwrap(), None);
    assert_eq!(setup.keychain.raw(VAULT_KEY), None, "没东西可存时不建条目");
}
