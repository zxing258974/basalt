use std::path::PathBuf;
use std::sync::Arc;

use basalt_core::config::{ConnectionConfig, ConnectionList, ConnectionRepository};
use basalt_core::llm::config::LlmRepository;
use basalt_core::query::history::HistoryStore;
use basalt_core::query::saved::SavedQueryStore;
use basalt_core::secret::{SecretStore, password_key};
use basalt_core::session::SessionManager;
use basalt_core::workspace::WorkspaceStore;
use basalt_core::{CoreError, CoreResult};
use tokio::sync::{Mutex, MutexGuard};

use crate::service::llm::LlmChats;
use crate::service::transfer::TransferJobs;

pub struct AppState {
    repo: ConnectionRepository,
    /// 串行化「读 → 改钥匙串 → 写配置」,避免并发操作互相覆盖。
    write_lock: Mutex<()>,
    pub secrets: Arc<dyn SecretStore>,
    pub sessions: SessionManager,
    /// 执行历史,和连接配置放在同一个目录。
    pub history: HistoryStore,
    /// 保存的查询,在配置目录的 queries/ 下。
    pub saved_queries: SavedQueryStore,
    /// 正在进行的导入导出,取消时按任务 id 找。
    pub transfers: TransferJobs,
    /// SQL 助手的模型配置(llm-models.json,不含 API Key)和它的写锁。
    pub llm: LlmRepository,
    pub llm_lock: Mutex<()>,
    /// 正在进行的助手对话,取消时按请求 id 找。
    pub chats: LlmChats,
    /// 上次开着的标签页(workspace.json),下次启动时恢复。
    pub workspace: WorkspaceStore,
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
        let dir = config_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        Self {
            repo: ConnectionRepository::new(config_path),
            write_lock: Mutex::new(()),
            secrets,
            sessions: SessionManager::default(),
            history: HistoryStore::new(dir.join("history.jsonl")),
            saved_queries: SavedQueryStore::new(dir.join("queries")),
            transfers: TransferJobs::default(),
            llm: LlmRepository::new(dir.join("llm-models.json")),
            llm_lock: Mutex::new(()),
            chats: LlmChats::default(),
            workspace: WorkspaceStore::new(dir.join("workspace.json")),
        }
    }

    /// 只有用只读目录造写入失败的测试用,那些测试只在 Unix 上跑。
    #[cfg(all(test, unix))]
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

    /// 只有存过密码的连接才去读钥匙串:没存的(SQLite、每次手输)读了也是空,还可能弹授权框。
    pub fn saved_password(&self, config: &ConnectionConfig) -> CoreResult<Option<String>> {
        if !config.save_password {
            return Ok(None);
        }
        self.secrets.get(&password_key(&config.id))
    }
}
