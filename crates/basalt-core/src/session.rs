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
    pub async fn close(&self, id: &str) -> bool {
        let removed = {
            let mut registry = self.registry.write().await;
            *registry.generations.entry(id.to_owned()).or_default() += 1;
            registry.sessions.remove(id)
        };
        match removed {
            Some(session) => {
                session.close().await;
                true
            }
            None => false,
        }
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
