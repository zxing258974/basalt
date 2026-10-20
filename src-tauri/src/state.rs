use std::path::PathBuf;
use std::sync::Arc;

use basalt_core::config::{ConnectionConfig, ConnectionList, ConnectionRepository};
use basalt_core::secret::{SecretStore, password_key};
use basalt_core::session::SessionManager;
use basalt_core::{CoreError, CoreResult};
use tokio::sync::{Mutex, MutexGuard};

pub struct AppState {
    repo: ConnectionRepository,
    /// 串行化「读 → 改钥匙串 → 写配置」,避免并发操作互相覆盖。
    write_lock: Mutex<()>,
    pub secrets: Arc<dyn SecretStore>,
    pub sessions: SessionManager,
}

/// 持有写锁期间对配置文件的读写。锁随它一起释放。
pub struct Store<'a> {
    repo: &'a ConnectionRepository,
    _guard: MutexGuard<'a, ()>,
}

impl Store<'_> {
    pub fn load(&self) -> CoreResult<ConnectionList> {
        self.repo.load()
    }

    pub fn save(&self, list: &ConnectionList) -> CoreResult<()> {
        self.repo.save(list)
    }
}

impl AppState {
    pub fn new(config_path: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            repo: ConnectionRepository::new(config_path),
            write_lock: Mutex::new(()),
            secrets,
            sessions: SessionManager::default(),
        }
    }

    #[cfg(test)]
    pub fn config_dir(&self) -> &std::path::Path {
        self.repo
            .path()
            .parent()
            .unwrap_or(std::path::Path::new("."))
    }

    pub fn load(&self) -> CoreResult<ConnectionList> {
        self.repo.load()
    }

    pub fn find(&self, id: &str) -> CoreResult<ConnectionConfig> {
        self.load()?
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::ConnectionNotFound(id.to_owned()))
    }

    /// 拿到写锁后才能改配置。
    pub async fn store(&self) -> Store<'_> {
        Store {
            repo: &self.repo,
            _guard: self.write_lock.lock().await,
        }
    }

    pub fn saved_password(&self, id: &str) -> CoreResult<Option<String>> {
        self.secrets.get(&password_key(id))
    }
}
