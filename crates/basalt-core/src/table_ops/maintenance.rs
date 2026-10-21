//! 表的维护:MySQL 的 ANALYZE / CHECK / OPTIMIZE / REPAIR TABLE,PostgreSQL 的 VACUUM / ANALYZE / REINDEX,
//! SQLite 的 ANALYZE / VACUUM(VACUUM 是整个库)。
//!
//! MySQL 这几条语句以结果集返回消息(每行一条 Msg_type、Msg_text),原样交给前端;表不存在、引擎不支持
//! 也是这样报的,不算语句出错。PostgreSQL、SQLite 不返回消息,VACUUM、REINDEX 另外给出前后的大小
//! (PostgreSQL 是表连同索引,SQLite 是整个库文件),让人看得出有没有效果。PostgreSQL 的 VACUUM、ANALYZE
//! 在没有权限时只发警告(拿不到)就跳过,看着像成功了,所以先查权限。

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::session::Session;
use crate::sql::{Dialect, quote_ident, table_name};
use crate::table::TableRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Maintenance {
    Analyze,
    Check,
    Optimize,
    Repair,
    Vacuum,
    Reindex,
}

impl Maintenance {
    /// 这种数据库能做的,按菜单里的顺序。
    pub fn available(dialect: Dialect) -> &'static [Maintenance] {
        use Maintenance::*;
        match dialect {
            Dialect::Mysql => &[Analyze, Check, Optimize, Repair],
            Dialect::Postgres => &[Vacuum, Analyze, Reindex],
            Dialect::Sqlite => &[Analyze, Vacuum],
        }
    }

    fn keyword(self) -> &'static str {
        match self {
            Self::Analyze => "ANALYZE",
            Self::Check => "CHECK",
            Self::Optimize => "OPTIMIZE",
            Self::Repair => "REPAIR",
            Self::Vacuum => "VACUUM",
            Self::Reindex => "REINDEX",
        }
    }

    /// 执行前后要不要量一下大小。
    fn measures_size(self, dialect: Dialect) -> bool {
        dialect != Dialect::Mysql && matches!(self, Self::Vacuum | Self::Reindex)
    }
}

/// 数据库返回的一条消息。`level` 是 MySQL 的 Msg_type(小写):status、error、info、note、warning。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceMessage {
    pub level: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeChange {
    pub before: u64,
    pub after: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceOutcome {
    /// 执行的语句。
    pub sql: String,
    /// PostgreSQL、SQLite 不返回消息,为空。
    pub messages: Vec<MaintenanceMessage>,
    /// 只有 PostgreSQL 的 VACUUM、REINDEX 和 SQLite 的 VACUUM 才有。
    pub size: Option<SizeChange>,
    pub elapsed_ms: u64,
}

/// 要执行的语句;这种数据库没有这项维护时报 Unsupported。
pub fn maintenance_sql(
    dialect: Dialect,
    table: &TableRef,
    operation: Maintenance,
) -> CoreResult<String> {
    use Maintenance::*;
    if !Maintenance::available(dialect).contains(&operation) {
        let product = match dialect {
            Dialect::Mysql => "MySQL",
            Dialect::Postgres => "PostgreSQL",
            Dialect::Sqlite => "SQLite",
        };
        return Err(CoreError::Unsupported(format!(
            "{product} 没有 {} 这项维护",
            operation.keyword()
        )));
    }
    let name = table_name(dialect, table);
    Ok(match (dialect, operation) {
        (Dialect::Mysql, Analyze) => format!("ANALYZE TABLE {name}"),
        (Dialect::Mysql, Check) => format!("CHECK TABLE {name}"),
        (Dialect::Mysql, Optimize) => format!("OPTIMIZE TABLE {name}"),
        (Dialect::Mysql, Repair) => format!("REPAIR TABLE {name}"),
        (Dialect::Postgres, Vacuum) => format!("VACUUM {name}"),
        (Dialect::Postgres, Reindex) => format!("REINDEX TABLE {name}"),
        (Dialect::Sqlite, Vacuum) => format!("VACUUM {}", quote_ident(dialect, &table.database)),
        (_, Analyze) => format!("ANALYZE {name}"),
        _ => unreachable!("上面已经按 available 拦下"),
    })
}

impl Session {
    pub async fn maintain_table(
        &self,
        table: &TableRef,
        operation: Maintenance,
    ) -> CoreResult<MaintenanceOutcome> {
        self.check_table(table)?;
        let dialect = self.dialect();
        let sql = maintenance_sql(dialect, table, operation)?;
        if let Self::Postgres(s) = self
            && matches!(operation, Maintenance::Vacuum | Maintenance::Analyze)
            && !s.can_maintain(table).await?
        {
            return Err(CoreError::Database(format!(
                "没有权限对「{}」做 {}：要是表的所有者、库的所有者或超级用户（PostgreSQL 17 起也可以授予 MAINTAIN 权限）。\
                 没有权限时数据库只给一条警告就跳过这张表，所以这里先拦下来。",
                table.table,
                operation.keyword()
            )));
        }
        let measures = operation.measures_size(dialect);
        let before = match measures {
            true => Some(self.storage_size(table).await?),
            false => None,
        };
        let started = Instant::now();
        let messages = match self {
            Self::Mysql(s) => s.maintenance_messages(&sql).await?,
            Self::Postgres(s) => {
                s.execute_outside_transaction(&table.database, &sql).await?;
                Vec::new()
            }
            Self::Sqlite(s) => {
                s.execute_outside_transaction(&sql).await?;
                Vec::new()
            }
        };
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let size = match before {
            Some(before) => Some(SizeChange {
                before,
                after: self.storage_size(table).await?,
            }),
            None => None,
        };
        Ok(MaintenanceOutcome {
            sql,
            messages,
            size,
            elapsed_ms,
        })
    }

    /// PostgreSQL:表连同索引、TOAST 的大小;SQLite:整个库文件。MySQL 不量(统计信息有缓存,前后看不出变化)。
    async fn storage_size(&self, table: &TableRef) -> CoreResult<u64> {
        match self {
            Self::Postgres(s) => s.total_relation_size(table).await,
            Self::Sqlite(s) => s.database_size(&table.database).await,
            Self::Mysql(_) => Err(CoreError::Unsupported("MySQL 不量表的大小".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(dialect: Dialect) -> TableRef {
        TableRef {
            database: "shop".into(),
            schema: (dialect == Dialect::Postgres).then(|| "public".into()),
            table: "users".into(),
        }
    }

    #[test]
    fn statements_follow_each_dialect() {
        use Maintenance::*;
        let sql = |dialect, op| maintenance_sql(dialect, &table(dialect), op).unwrap();
        assert_eq!(sql(Dialect::Mysql, Check), "CHECK TABLE `shop`.`users`");
        assert_eq!(sql(Dialect::Mysql, Repair), "REPAIR TABLE `shop`.`users`");
        assert_eq!(sql(Dialect::Mysql, Analyze), "ANALYZE TABLE `shop`.`users`");
        assert_eq!(
            sql(Dialect::Mysql, Optimize),
            "OPTIMIZE TABLE `shop`.`users`"
        );
        assert_eq!(
            sql(Dialect::Postgres, Vacuum),
            "VACUUM \"public\".\"users\""
        );
        assert_eq!(
            sql(Dialect::Postgres, Analyze),
            "ANALYZE \"public\".\"users\""
        );
        assert_eq!(
            sql(Dialect::Postgres, Reindex),
            "REINDEX TABLE \"public\".\"users\""
        );
        assert_eq!(sql(Dialect::Sqlite, Analyze), "ANALYZE \"shop\".\"users\"");
        assert_eq!(sql(Dialect::Sqlite, Vacuum), "VACUUM \"shop\"");
    }

    #[test]
    fn operations_a_database_lacks_are_refused() {
        let error = maintenance_sql(
            Dialect::Sqlite,
            &table(Dialect::Sqlite),
            Maintenance::Repair,
        )
        .unwrap_err();
        assert_eq!(error.code(), "unsupported");
        assert!(
            maintenance_sql(
                Dialect::Postgres,
                &table(Dialect::Postgres),
                Maintenance::Optimize
            )
            .is_err()
        );
        assert!(
            maintenance_sql(Dialect::Mysql, &table(Dialect::Mysql), Maintenance::Vacuum).is_err()
        );
        assert!(Maintenance::Vacuum.measures_size(Dialect::Sqlite));
        assert!(!Maintenance::Optimize.measures_size(Dialect::Mysql));
        assert!(!Maintenance::Analyze.measures_size(Dialect::Postgres));
    }

    #[test]
    fn outcome_serializes_in_camel_case() {
        let outcome = MaintenanceOutcome {
            sql: "VACUUM \"main\"".into(),
            messages: vec![MaintenanceMessage {
                level: "status".into(),
                text: "OK".into(),
            }],
            size: Some(SizeChange {
                before: 8192,
                after: 4096,
            }),
            elapsed_ms: 12,
        };
        assert_eq!(
            serde_json::to_value(outcome).unwrap(),
            serde_json::json!({
                "sql": "VACUUM \"main\"",
                "messages": [{ "level": "status", "text": "OK" }],
                "size": { "before": 8192, "after": 4096 },
                "elapsedMs": 12,
            })
        );
        assert_eq!(
            serde_json::from_value::<Maintenance>(serde_json::json!("reindex")).unwrap(),
            Maintenance::Reindex
        );
    }
}
