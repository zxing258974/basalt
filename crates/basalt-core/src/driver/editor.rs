//! 查询标签页专用连接的三种实现统一成一个类型,以及取消正在执行的语句。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sqlx::mysql::MySqlPool;
use sqlx::postgres::PgPool;

use super::mysql_query::{self, MysqlEditor};
use super::postgres_query::{self, PgEditor};
use super::sqlite_query::SqliteEditor;
use crate::error::CoreResult;
use crate::query::collect::Collected;
use crate::query::{QueryScope, ResultColumn};
use crate::session::Session;

pub(crate) enum EditorConnection {
    Mysql(MysqlEditor),
    Postgres(PgEditor),
    Sqlite(SqliteEditor),
}

/// 取消一条连接上正在执行的语句要用的东西。MySQL、PostgreSQL 得另开连接去说,SQLite 只要亮一下标志。
#[derive(Clone)]
pub(crate) enum CancelHandle {
    Mysql { pool: MySqlPool, thread_id: u64 },
    Postgres { pool: PgPool, pid: i32 },
    Sqlite { flag: Arc<AtomicBool> },
}

impl CancelHandle {
    pub(crate) async fn cancel(&self) -> CoreResult<()> {
        match self {
            Self::Mysql { pool, thread_id } => mysql_query::kill_query(pool, *thread_id).await,
            Self::Postgres { pool, pid } => postgres_query::cancel_backend(pool, *pid).await,
            Self::Sqlite { flag } => {
                flag.store(true, Ordering::Relaxed);
                Ok(())
            }
        }
    }
}

impl EditorConnection {
    /// 新开一条连接并切到 `scope`。返回连接和它实际所在的库 / schema。
    pub(crate) async fn open(
        session: &Session,
        scope: &QueryScope,
    ) -> CoreResult<(Self, QueryScope)> {
        let mut editor = match session {
            Session::Mysql(s) => Self::Mysql(s.open_editor().await?),
            Session::Postgres(s) => Self::Postgres(s.open_editor(scope.database.as_deref()).await?),
            Session::Sqlite(s) => Self::Sqlite(s.open_editor().await?),
        };
        let initial = editor.current_scope(scope).await;
        let actual = editor.switch(&initial, scope).await?;
        Ok((editor, actual))
    }

    /// PostgreSQL 一条连接只能在一个库里,换库得重开;其余都能就地切换。
    pub(crate) fn can_switch_to(&self, scope: &QueryScope) -> bool {
        match (self, scope.database.as_deref()) {
            (Self::Postgres(editor), Some(database)) => editor.database() == database,
            _ => true,
        }
    }

    /// 从 `current` 切到 `wanted`:MySQL 执行 USE,PostgreSQL 改 search_path。
    /// `wanted` 里没给的部分保持不变。
    pub(crate) async fn switch(
        &mut self,
        current: &QueryScope,
        wanted: &QueryScope,
    ) -> CoreResult<QueryScope> {
        match self {
            Self::Mysql(editor) => {
                if let Some(database) = &wanted.database
                    && current.database.as_ref() != Some(database)
                {
                    editor.use_database(database).await?;
                }
                Ok(QueryScope {
                    database: wanted.database.clone().or_else(|| current.database.clone()),
                    schema: None,
                })
            }
            Self::Postgres(editor) => {
                if let Some(schema) = &wanted.schema
                    && current.schema.as_ref() != Some(schema)
                {
                    editor.set_schema(schema).await?;
                }
                Ok(QueryScope {
                    database: Some(editor.database().to_owned()),
                    schema: wanted.schema.clone().or_else(|| current.schema.clone()),
                })
            }
            // SQLite 的「库」是 ATTACH 的别名,不加限定的名字本来就按顺序在所有库里找。
            Self::Sqlite(_) => Ok(QueryScope {
                database: wanted.database.clone().or_else(|| current.database.clone()),
                schema: None,
            }),
        }
    }

    /// 连接实际所在的库 / schema;读不出来时用 `fallback`。
    pub(crate) async fn current_scope(&mut self, fallback: &QueryScope) -> QueryScope {
        let read = match self {
            Self::Mysql(editor) => editor.current_database().await.map(|database| QueryScope {
                database,
                schema: None,
            }),
            Self::Postgres(editor) => editor.current_schema().await.map(|schema| QueryScope {
                database: Some(editor.database().to_owned()),
                schema,
            }),
            Self::Sqlite(_) => Ok(fallback.clone()),
        };
        read.unwrap_or_else(|error| {
            log::warn!("读取查询连接所在的库失败：{error}");
            fallback.clone()
        })
    }

    pub(crate) async fn cancel_handle(&self, session: &Session) -> CoreResult<CancelHandle> {
        Ok(match (self, session) {
            (Self::Mysql(editor), Session::Mysql(s)) => CancelHandle::Mysql {
                pool: s.cancel_pool(),
                thread_id: editor.thread_id(),
            },
            (Self::Postgres(editor), Session::Postgres(s)) => CancelHandle::Postgres {
                pool: s.cancel_pool().await?,
                pid: editor.pid(),
            },
            _ => CancelHandle::Sqlite {
                flag: Arc::new(AtomicBool::new(false)),
            },
        })
    }

    pub(crate) async fn run(&mut self, sql: &str, cancel: &CancelHandle) -> Collected {
        match self {
            Self::Mysql(editor) => editor.run(sql).await,
            Self::Postgres(editor) => editor.run(sql).await,
            Self::Sqlite(editor) => {
                let flag = match cancel {
                    CancelHandle::Sqlite { flag } => flag.clone(),
                    _ => Arc::new(AtomicBool::new(false)),
                };
                editor.run(sql, flag).await
            }
        }
    }

    pub(crate) async fn describe(&mut self, sql: &str) -> Vec<ResultColumn> {
        match self {
            Self::Mysql(editor) => editor.describe(sql).await,
            Self::Postgres(editor) => editor.describe(sql).await,
            Self::Sqlite(editor) => editor.describe(sql).await,
        }
    }

    /// SQLite 的「影响行数」对查询、DDL 也会报上一次增删改的数字,只有增删改的才可信。
    pub(crate) fn changes_only_for_dml(&self) -> bool {
        matches!(self, Self::Sqlite(_))
    }

    /// 丢掉这条连接上缓存的预备语句(表结构变了之后,缓存里的列信息是旧的)。
    pub(crate) async fn clear_cached_statements(&mut self) {
        let result = match self {
            Self::Mysql(editor) => editor.clear_cached_statements().await,
            Self::Postgres(editor) => editor.clear_cached_statements().await,
            Self::Sqlite(editor) => editor.clear_cached_statements().await,
        };
        if let Err(error) = result {
            log::warn!("清掉预备语句缓存失败：{error}");
        }
    }

    pub(crate) async fn ping(&mut self) -> bool {
        match self {
            Self::Mysql(editor) => editor.ping().await,
            Self::Postgres(editor) => editor.ping().await,
            Self::Sqlite(editor) => editor.ping().await,
        }
    }

    pub(crate) async fn close(self) {
        match self {
            Self::Mysql(editor) => editor.close().await,
            Self::Postgres(editor) => editor.close().await,
            Self::Sqlite(editor) => editor.close().await,
        }
    }
}
