//! 已打开的连接。`Session` 把三种驱动统一成一套接口,
//! `SessionManager` 按连接 id 管理它们的生命周期。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::DriverKind;
use crate::driver::mysql::MysqlSession;
use crate::driver::postgres::PgSession;
use crate::driver::sqlite::SqliteSession;
use crate::driver::{ConnectParams, ServerInfo, TableInfo};
use crate::error::{CoreError, CoreResult};
use crate::query::lease::EditorLease;
use crate::query::results::ResultSort;
use crate::query::{EditorRegistry, QueryScope, StatementOutcome};
use crate::value::CellValue;

pub enum Session {
    Mysql(MysqlSession),
    /// PgSession 带着连接参数和按库的池表,体积大,装箱。
    Postgres(Box<PgSession>),
    Sqlite(SqliteSession),
}

impl Session {
    pub async fn open(params: &ConnectParams) -> CoreResult<Self> {
        Ok(match params.driver {
            DriverKind::Mysql | DriverKind::Mariadb => {
                Self::Mysql(MysqlSession::open(params).await?)
            }
            DriverKind::Postgres => Self::Postgres(Box::new(PgSession::open(params).await?)),
            DriverKind::Sqlite => Self::Sqlite(SqliteSession::open(params).await?),
        })
    }

    pub fn server(&self) -> &ServerInfo {
        match self {
            Self::Mysql(s) => s.server(),
            Self::Postgres(s) => s.server(),
            Self::Sqlite(s) => s.server(),
        }
    }

    pub async fn list_databases(&self) -> CoreResult<Vec<String>> {
        match self {
            Self::Mysql(s) => s.list_databases().await,
            Self::Postgres(s) => s.list_databases().await,
            Self::Sqlite(s) => s.list_databases().await,
        }
    }

    /// 只有 PostgreSQL 有库下的 schema 层级,其余返回空列表。
    pub async fn list_schemas(&self, database: &str) -> CoreResult<Vec<String>> {
        match self {
            Self::Postgres(s) => s.list_schemas(database).await,
            Self::Mysql(_) | Self::Sqlite(_) => Ok(Vec::new()),
        }
    }

    pub async fn list_tables(
        &self,
        database: &str,
        schema: Option<&str>,
    ) -> CoreResult<Vec<TableInfo>> {
        match self {
            Self::Mysql(s) => s.list_tables(database).await,
            Self::Postgres(s) => {
                let schema = schema
                    .ok_or_else(|| CoreError::Unsupported("PostgreSQL 需要指定 schema".into()))?;
                s.list_tables(database, schema).await
            }
            Self::Sqlite(s) => s.list_tables(database).await,
        }
    }

    pub async fn close(&self) {
        match self {
            Self::Mysql(s) => s.close().await,
            Self::Postgres(s) => s.close().await,
            Self::Sqlite(s) => s.close().await,
        }
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let server = self.server();
        write!(f, "Session({} {})", server.product, server.version)
    }
}

/// 打开、读取版本后立即关闭,用于连接对话框的「测试连接」。
pub async fn test_connection(params: &ConnectParams) -> CoreResult<ServerInfo> {
    let session = Session::open(params).await?;
    let server = session.server().clone();
    session.close().await;
    Ok(server)
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    /// 每个连接 id 的代际号,每次 close 加一。打开前记下代际,
    /// 打开完成时代际变了,说明期间配置被改或连接被删,新会话作废。
    generations: HashMap<String, u64>,
}

impl Registry {
    fn generation(&self, id: &str) -> u64 {
        self.generations.get(id).copied().unwrap_or_default()
    }
}

/// [`SessionManager::begin_open`] 的结果:已有会话,或开始新建所需的代际票据。
pub(crate) enum OpenStart {
    Existing(Arc<Session>),
    Ticket(u64),
}

#[derive(Default)]
pub struct SessionManager {
    registry: RwLock<Registry>,
    /// 查询标签页的专用连接,随所属连接一起关闭。
    editors: EditorRegistry,
}

impl SessionManager {
    /// 已打开则直接返回现有会话,不会重复建连接。
    pub async fn open(&self, id: &str, params: &ConnectParams) -> CoreResult<Arc<Session>> {
        let generation = match self.begin_open(id).await {
            OpenStart::Existing(session) => return Ok(session),
            OpenStart::Ticket(generation) => generation,
        };
        let session = Arc::new(Session::open(params).await?);
        self.finish_open(id, generation, session).await
    }

    pub(crate) async fn begin_open(&self, id: &str) -> OpenStart {
        let registry = self.registry.read().await;
        match registry.sessions.get(id) {
            Some(existing) => OpenStart::Existing(existing.clone()),
            None => OpenStart::Ticket(registry.generation(id)),
        }
    }

    pub(crate) async fn finish_open(
        &self,
        id: &str,
        generation: u64,
        session: Arc<Session>,
    ) -> CoreResult<Arc<Session>> {
        let mut registry = self.registry.write().await;
        if registry.generation(id) != generation {
            drop(registry);
            session.close().await;
            return Err(CoreError::Superseded);
        }
        // 并发打开同一连接时,先写入的那个胜出,后来的关掉自己。
        if let Some(existing) = registry.sessions.get(id) {
            let existing = existing.clone();
            drop(registry);
            session.close().await;
            return Ok(existing);
        }
        registry.sessions.insert(id.to_owned(), session.clone());
        Ok(session)
    }

    pub async fn get(&self, id: &str) -> CoreResult<Arc<Session>> {
        self.registry
            .read()
            .await
            .sessions
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::NotConnected(id.to_owned()))
    }

    pub async fn is_open(&self, id: &str) -> bool {
        self.registry.read().await.sessions.contains_key(id)
    }

    pub async fn open_ids(&self) -> Vec<String> {
        self.registry
            .read()
            .await
            .sessions
            .keys()
            .cloned()
            .collect()
    }

    /// 关掉会话并作废正在进行中的打开。返回是否真的关掉了一个会话。
    /// 查询标签页的专用连接先断开:取消它们正在执行的语句还要用会话的连接池。
    pub async fn close(&self, id: &str) -> bool {
        let removed = {
            let mut registry = self.registry.write().await;
            *registry.generations.entry(id.to_owned()).or_default() += 1;
            registry.sessions.remove(id)
        };
        self.editors.close_connection(id).await;
        match removed {
            Some(session) => {
                session.close().await;
                true
            }
            None => false,
        }
    }
}

impl SessionManager {
    /// 在查询标签页 `editor_id` 的专用连接上执行一条语句,连接没打开时报错。
    pub async fn run_query(
        &self,
        editor_id: &str,
        connection_id: &str,
        scope: &QueryScope,
        sql: &str,
    ) -> CoreResult<StatementOutcome> {
        let session = self.get(connection_id).await?;
        self.editors
            .run(editor_id, connection_id, &session, scope, sql)
            .await
    }

    /// 取消查询标签页正在执行的语句。没有在执行时返回 false。
    pub async fn cancel_query(&self, editor_id: &str) -> CoreResult<bool> {
        self.editors.cancel(editor_id).await
    }

    /// 取查询结果里的一段行(往下滚、排序时)。
    pub async fn result_rows(
        &self,
        editor_id: &str,
        result_id: u64,
        offset: usize,
        limit: usize,
        sort: Option<ResultSort>,
    ) -> CoreResult<Vec<Vec<CellValue>>> {
        self.editors
            .result_rows(editor_id, result_id, offset, limit, sort)
            .await
    }

    /// 前端不再显示的结果,放掉它们占的内存。
    pub fn release_results(&self, editor_id: &str, result_ids: &[u64]) {
        self.editors.release_results(editor_id, result_ids);
    }

    /// 查询标签页关了,断开它的专用连接。
    pub async fn close_editor(&self, editor_id: &str) {
        self.editors.close(editor_id).await;
    }

    /// 导入导出借用 `editor_id` 的专用连接(查询标签页的,或者为这次导入导出新开的)。
    pub async fn lease_editor(
        &self,
        editor_id: &str,
        connection_id: &str,
        scope: &QueryScope,
    ) -> CoreResult<EditorLease> {
        let session = self.get(connection_id).await?;
        self.editors
            .lease(editor_id, connection_id, &session, scope)
            .await
    }

    #[cfg(test)]
    pub(crate) fn editor_count(&self) -> usize {
        self.editors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn sqlite_params(dir: &std::path::Path) -> ConnectParams {
        let path = dir.join("s.db");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        sqlx::SqlitePool::connect_with(options)
            .await
            .unwrap()
            .close()
            .await;
        ConnectParams {
            driver: DriverKind::Sqlite,
            host: String::new(),
            port: 0,
            username: String::new(),
            password: None,
            database: None,
            file_path: Some(path.to_string_lossy().into_owned()),
            ssl: crate::config::SslSettings::default(),
            ssh: None,
        }
    }

    #[tokio::test]
    async fn open_finishing_after_close_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let params = sqlite_params(dir.path()).await;
        let manager = SessionManager::default();

        let OpenStart::Ticket(generation) = manager.begin_open("c1").await else {
            panic!("还没打开,应该拿到票据");
        };
        // 连接还在建立时,配置被编辑或连接被删除。
        manager.close("c1").await;
        let late = Arc::new(Session::open(&params).await.unwrap());

        let error = manager
            .finish_open("c1", generation, late)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "superseded");
        assert!(!manager.is_open("c1").await);
    }

    #[tokio::test]
    async fn query_tabs_close_with_their_tab_or_connection() {
        let dir = tempfile::tempdir().unwrap();
        let params = sqlite_params(dir.path()).await;
        let manager = SessionManager::default();
        manager.open("c1", &params).await.unwrap();
        let scope = QueryScope::default();
        manager
            .run_query("tab", "c1", &scope, "SELECT 1")
            .await
            .unwrap();
        assert_eq!(manager.editor_count(), 1);
        manager.close_editor("tab").await;
        assert_eq!(manager.editor_count(), 0);

        manager
            .run_query("tab", "c1", &scope, "SELECT 1")
            .await
            .unwrap();
        manager.close("c1").await;
        assert_eq!(manager.editor_count(), 0);
    }

    #[tokio::test]
    async fn open_finishing_without_interference_is_registered() {
        let dir = tempfile::tempdir().unwrap();
        let params = sqlite_params(dir.path()).await;
        let manager = SessionManager::default();

        let OpenStart::Ticket(generation) = manager.begin_open("c1").await else {
            panic!("还没打开,应该拿到票据");
        };
        let session = Arc::new(Session::open(&params).await.unwrap());
        manager
            .finish_open("c1", generation, session)
            .await
            .unwrap();
        assert!(manager.is_open("c1").await);
        assert!(matches!(
            manager.begin_open("c1").await,
            OpenStart::Existing(_)
        ));
    }
}
