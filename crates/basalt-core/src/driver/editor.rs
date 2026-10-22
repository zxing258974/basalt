//! 查询标签页专用连接的三种实现统一成一个类型,以及取消正在执行的语句。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sqlx::mysql::MySqlPool;
use sqlx::postgres::PgPool;

use super::mysql_query::{self, MysqlEditor};
use super::postgres_query::{self, PgEditor};
use super::sqlite_query::SqliteEditor;
use crate::error::CoreResult;
use crate::query::collect::{Collected, CollectedSet, RawOrigin};
use crate::query::{ColumnSource, QueryScope};
use crate::session::Session;
use crate::table::TableRef;

/// 结果列的来源。
pub(crate) struct Sources {
    pub columns: Vec<Option<ColumnSource>>,
    /// 来源看得出、却不能在结果里改的原因(如来自临时表)。
    pub read_only: Option<String>,
}

fn temporary_reason(table: &str) -> String {
    format!("结果来自临时表 {table}，不能在这里改：提交修改走的是另一条连接，看不到这张临时表。")
}

fn distinct_tables(columns: &[Option<ColumnSource>]) -> Vec<TableRef> {
    let mut tables: Vec<TableRef> = Vec::new();
    for source in columns.iter().flatten() {
        if !tables.contains(&source.table) {
            tables.push(source.table.clone());
        }
    }
    tables
}

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

    pub(crate) async fn describe(&mut self, sql: &str) -> Option<CollectedSet> {
        match self {
            Self::Mysql(editor) => editor.describe(sql).await,
            Self::Postgres(editor) => editor.describe(sql).await,
            Self::Sqlite(editor) => editor.describe(sql).await,
        }
    }

    /// 把驱动报的来源换成库表名。查不出来就当不知道来源,结果只是不能改。
    /// 来自临时表的结果也不能改:提交修改走连接池里的另一条连接,看不到这个会话的临时表
    /// (MySQL 上同名的临时表还会让修改落到真表上)。
    pub(crate) async fn resolve_sources(
        &mut self,
        session: Option<&Session>,
        scope: &QueryScope,
        origins: &[RawOrigin],
    ) -> Sources {
        match self {
            Self::Mysql(editor) => {
                let columns: Vec<_> = origins
                    .iter()
                    .map(|origin| named_source(origin, scope.database.as_deref()))
                    .collect();
                let read_only = match editor.temporary_among(&distinct_tables(&columns)).await {
                    Ok(temporary) => temporary.map(|table| temporary_reason(&table)),
                    Err(error) => Some(format!(
                        "没能确认结果来自的表是不是临时表，不能在这里改：{error}"
                    )),
                };
                Sources { columns, read_only }
            }
            Self::Sqlite(_) => {
                let columns: Vec<_> = origins
                    .iter()
                    .map(|origin| named_source(origin, Some("main")))
                    .collect();
                let read_only = columns
                    .iter()
                    .flatten()
                    .find(|source| source.table.database == "temp")
                    .map(|source| temporary_reason(&source.table.table));
                Sources { columns, read_only }
            }
            Self::Postgres(editor) => {
                let relations: Vec<(u32, i16)> = origins
                    .iter()
                    .filter_map(|origin| match origin {
                        RawOrigin::Relation { oid, attnum } => Some((*oid, *attnum)),
                        _ => None,
                    })
                    .collect();
                let database = editor.database().to_owned();
                let names = match session {
                    Some(Session::Postgres(s)) if !relations.is_empty() => {
                        s.relation_names(&database, &relations).await
                    }
                    _ => Ok(Vec::new()),
                }
                .unwrap_or_else(|error| {
                    log::debug!("查结果列来自哪张表失败：{error}");
                    Vec::new()
                });
                let mut read_only = None;
                let columns = origins
                    .iter()
                    .map(|origin| {
                        let RawOrigin::Relation { oid, attnum } = origin else {
                            return None;
                        };
                        let (_, (schema, table, column, temporary)) =
                            names.iter().find(|(key, _)| *key == (*oid, *attnum))?;
                        if *temporary {
                            read_only = Some(temporary_reason(table));
                        }
                        Some(ColumnSource {
                            table: TableRef {
                                database: database.clone(),
                                schema: Some(schema.clone()),
                                table: table.clone(),
                            },
                            column: column.clone(),
                        })
                    })
                    .collect();
                Sources { columns, read_only }
            }
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

/// MySQL 报「库.表」(库名、表名里都不会有点);SQLite 主库只报表名、附加库报「库.表」。
/// 没带库名时用 `default_database`(MySQL 是当前库,SQLite 是 main)。
fn named_source(origin: &RawOrigin, default_database: Option<&str>) -> Option<ColumnSource> {
    let RawOrigin::Named { table, column } = origin else {
        return None;
    };
    let (database, table) = match table.split_once('.') {
        Some((database, table)) => (database.to_owned(), table.to_owned()),
        None => (default_database?.to_owned(), table.clone()),
    };
    Some(ColumnSource {
        table: TableRef {
            database,
            schema: None,
            table,
        },
        column: column.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(table: &str, column: &str) -> RawOrigin {
        RawOrigin::Named {
            table: table.into(),
            column: column.into(),
        }
    }

    fn source(database: &str, table: &str, column: &str) -> Option<ColumnSource> {
        Some(ColumnSource {
            table: TableRef {
                database: database.into(),
                schema: None,
                table: table.into(),
            },
            column: column.into(),
        })
    }

    #[test]
    fn named_sources_split_off_the_database() {
        assert_eq!(
            named_source(&named("shop.orders", "id"), Some("other")),
            source("shop", "orders", "id")
        );
        assert_eq!(
            named_source(&named("orders", "id"), Some("main")),
            source("main", "orders", "id")
        );
        assert_eq!(named_source(&named("orders", "id"), None), None);
        assert_eq!(named_source(&RawOrigin::None, Some("main")), None);
    }
}
