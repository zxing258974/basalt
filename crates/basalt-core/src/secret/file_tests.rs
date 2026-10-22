use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::*;
use crate::secret::{MemorySecretStore, QuietDelete};

/// 假的钥匙串:记下每个键被读了几次,可以让它对某些键拒绝放行。
#[derive(Default)]
struct Keychain {
    items: MemorySecretStore,
    reads: Mutex<HashMap<String, usize>>,
    refused: Mutex<HashSet<String>>,
}

#[derive(Clone, Default)]
struct Legacy(Arc<Keychain>);

impl Legacy {
    fn with(key: &str, secret: &str) -> Self {
        let legacy = Self::default();
        legacy.0.items.set(key, secret).unwrap();
        legacy
    }
    fn reads(&self, key: &str) -> usize {
        self.0.reads.lock().unwrap().get(key).copied().unwrap_or(0)
    }
    fn refuse(&self, key: &str, refuse: bool) {
        let mut refused = self.0.refused.lock().unwrap();
        if refuse {
            refused.insert(key.to_owned());
        } else {
            refused.remove(key);
        }
    }
}

impl SecretStore for Legacy {
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        *self
            .0
            .reads
            .lock()
            .unwrap()
            .entry(key.to_owned())
            .or_default() += 1;
        if self.0.refused.lock().unwrap().contains(key) {
            return Err(CoreError::Secret("用户拒绝访问钥匙串".into()));
        }
        self.0.items.get(key)
    }
    fn set(&self, _key: &str, _secret: &str) -> CoreResult<()> {
        panic!("不该往钥匙串里写");
    }
    fn delete(&self, _key: &str) -> CoreResult<()> {
        panic!("不该删钥匙串里的条目");
    }
}

struct Setup {
    dir: tempfile::TempDir,
    legacy: Legacy,
}

impl Setup {
    fn new(legacy: Legacy) -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            legacy,
        }
    }
    fn path(&self) -> PathBuf {
        self.dir.path().join("config/secrets.json")
    }
    /// 一次运行:新开一个,内存里什么都没有,和重新启动 Basalt 一样。
    fn run(&self) -> FileSecretStore {
        FileSecretStore::new(self.path()).with_legacy(Box::new(self.legacy.clone()))
    }
}

#[test]
fn keeps_secrets_in_a_file_only_this_user_can_read() {
    let setup = Setup::new(Legacy::default());
    let store = setup.run();
    store.set("connection:a:password", "pa").unwrap();
    store.set("llm:m:api_key", "sk").unwrap();
    let next = setup.run();
    assert_eq!(
        next.get("connection:a:password").unwrap().as_deref(),
        Some("pa")
    );
    assert_eq!(next.get("llm:m:api_key").unwrap().as_deref(), Some("sk"));

    let names: Vec<_> = std::fs::read_dir(setup.path().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["secrets.json"], "没有留下临时文件");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(setup.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn a_secret_moves_over_from_the_keychain_once() {
    let setup = Setup::new(Legacy::with("connection:a:password", "old"));
    assert_eq!(
        setup.run().get("connection:a:password").unwrap().as_deref(),
        Some("old")
    );
    for _ in 0..3 {
        assert_eq!(
            setup.run().get("connection:a:password").unwrap().as_deref(),
            Some("old")
        );
    }
    assert_eq!(
        setup.legacy.reads("connection:a:password"),
        1,
        "只问了钥匙串一次"
    );
}

#[test]
fn a_key_missing_from_the_keychain_is_not_asked_about_again() {
    let setup = Setup::new(Legacy::default());
    assert_eq!(setup.run().get("connection:x:password").unwrap(), None);
    assert_eq!(setup.run().get("connection:x:password").unwrap(), None);
    assert_eq!(setup.legacy.reads("connection:x:password"), 1);
}

#[test]
fn a_deleted_secret_does_not_come_back_from_the_keychain() {
    let setup = Setup::new(Legacy::with("connection:a:password", "old"));
    let store = setup.run();
    store.delete("connection:a:password").unwrap();
    assert_eq!(setup.run().get("connection:a:password").unwrap(), None);
    assert_eq!(setup.legacy.reads("connection:a:password"), 0);

    store.set("connection:a:password", "new").unwrap();
    assert_eq!(
        setup.run().get("connection:a:password").unwrap().as_deref(),
        Some("new")
    );
    assert_eq!(
        store.delete_quietly("connection:a:password").unwrap(),
        QuietDelete::Removed
    );
    assert_eq!(setup.run().get("connection:a:password").unwrap(), None);
}

#[test]
fn a_refused_keychain_read_is_asked_again_next_time() {
    let setup = Setup::new(Legacy::with("connection:a:password", "old"));
    setup.legacy.refuse("connection:a:password", true);
    assert!(setup.run().get("connection:a:password").is_err());
    setup.legacy.refuse("connection:a:password", false);
    assert_eq!(
        setup.run().get("connection:a:password").unwrap().as_deref(),
        Some("old")
    );
}

#[test]
fn a_file_it_cannot_read_is_left_alone() {
    let setup = Setup::new(Legacy::default());
    std::fs::create_dir_all(setup.path().parent().unwrap()).unwrap();
    std::fs::write(setup.path(), "{写到一半").unwrap();
    let store = setup.run();
    let error = store.get("connection:a:password").unwrap_err().to_string();
    assert!(error.contains("读不懂"), "{error}");
    assert!(store.set("connection:a:password", "pw").is_err());
    assert_eq!(std::fs::read_to_string(setup.path()).unwrap(), "{写到一半");
}

#[test]
fn without_a_keychain_nothing_is_recorded_for_missing_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let store = FileSecretStore::new(&path);
    assert_eq!(store.get("connection:a:password").unwrap(), None);
    store.delete("connection:a:password").unwrap();
    assert!(!path.exists(), "没东西要记时不建文件");
}

fn keys(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn everything_moves_over_in_one_go_and_the_keychain_is_left_alone_afterwards() {
    let legacy = Legacy::with("connection:a:password", "pa");
    legacy.0.items.set("llm:m:api_key", "sk").unwrap();
    let setup = Setup::new(legacy);
    let wanted = keys(&[
        "connection:a:password",
        "llm:m:api_key",
        "connection:gone:password",
    ]);
    assert_eq!(setup.run().migrate_legacy(&wanted).unwrap(), 2);

    let next = setup.run();
    assert_eq!(
        next.get("connection:a:password").unwrap().as_deref(),
        Some("pa")
    );
    assert_eq!(next.get("llm:m:api_key").unwrap().as_deref(), Some("sk"));
    // 钥匙串里没有的、清单以外的,搬完以后都不再去问钥匙串。
    assert_eq!(next.get("connection:gone:password").unwrap(), None);
    assert_eq!(next.get("connection:new:password").unwrap(), None);
    assert_eq!(setup.legacy.reads("connection:gone:password"), 1);
    assert_eq!(setup.legacy.reads("connection:new:password"), 0);
    assert_eq!(setup.legacy.reads("connection:a:password"), 1);
    // 再启动一次也不再搬。
    assert_eq!(setup.run().migrate_legacy(&wanted).unwrap(), 0);
    assert_eq!(setup.legacy.reads("connection:a:password"), 1);
}

#[test]
fn keys_already_in_the_file_are_not_read_again() {
    let setup = Setup::new(Legacy::with("connection:a:password", "old"));
    setup.run().set("connection:a:password", "new").unwrap();
    assert_eq!(
        setup
            .run()
            .migrate_legacy(&keys(&["connection:a:password"]))
            .unwrap(),
        0
    );
    assert_eq!(setup.legacy.reads("connection:a:password"), 0);
    assert_eq!(
        setup.run().get("connection:a:password").unwrap().as_deref(),
        Some("new")
    );
}

#[test]
fn a_refused_move_keeps_what_it_got_and_never_asks_again() {
    let legacy = Legacy::with("connection:a:password", "pa");
    legacy.0.items.set("connection:b:password", "pb").unwrap();
    legacy.0.items.set("connection:c:password", "pc").unwrap();
    legacy.refuse("connection:b:password", true);
    let setup = Setup::new(legacy);
    let wanted = keys(&[
        "connection:a:password",
        "connection:b:password",
        "connection:c:password",
    ]);
    assert!(setup.run().migrate_legacy(&wanted).is_err());

    // 拒绝过一次就不再问:每次启动都弹框正是用户不要的。没搬过来的要用户重新填。
    setup.legacy.refuse("connection:b:password", false);
    let next = setup.run();
    assert_eq!(next.migrate_legacy(&wanted).unwrap(), 0);
    assert_eq!(
        next.get("connection:a:password").unwrap().as_deref(),
        Some("pa")
    );
    assert_eq!(next.get("connection:b:password").unwrap(), None);
    assert_eq!(next.get("connection:c:password").unwrap(), None);
    assert_eq!(setup.legacy.reads("connection:b:password"), 1);
    assert_eq!(setup.legacy.reads("connection:c:password"), 0);
}

#[test]
fn after_the_move_deleting_needs_no_tombstones() {
    let setup = Setup::new(Legacy::with("connection:a:password", "pa"));
    let store = setup.run();
    store
        .migrate_legacy(&keys(&["connection:a:password"]))
        .unwrap();
    store.delete("connection:a:password").unwrap();
    let text = std::fs::read_to_string(setup.path()).unwrap();
    assert!(!text.contains("connection:a:password"), "{text}");
    assert_eq!(setup.run().get("connection:a:password").unwrap(), None);
}

#[test]
fn without_a_keychain_there_is_nothing_to_move() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let store = FileSecretStore::new(&path);
    assert_eq!(
        store
            .migrate_legacy(&keys(&["connection:a:password"]))
            .unwrap(),
        0
    );
    assert!(!path.exists());
}
