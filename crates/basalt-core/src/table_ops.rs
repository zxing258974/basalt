//! 表级操作:删除表、复制表(只复制结构,或连数据一起)、清空与截断、重命名;维护见 [`maintenance`]。
//!
//! 复制表按各家的原生写法建新表,结构尽量原样:
//! - MySQL:`CREATE TABLE 新 LIKE 旧`,带上字段、索引、表选项(不含外键、触发器)。DDL 会隐式提交,
//!   复制数据失败时把刚建的新表删掉。
//! - PostgreSQL:`LIKE 旧 INCLUDING ALL`(默认值、约束、索引、标识列、注释;不含外键、触发器),
//!   整个过程一个事务;复制数据后把新表标识列的序列推到最大值。serial 列的默认值仍指向旧表的序列。
//! - SQLite:没有 LIKE,把 `sqlite_master` 里的建表、建索引语句换上新名字执行(见 [`sqlite_copy`]),一个事务。
//!
//! 清空(`DELETE FROM`)逐行删、出错整体回滚、自增计数保留;截断(`TRUNCATE`)快、自增计数从头开始,
//! MySQL 的截断会隐式提交、不能回滚,被别的表的外键引用时两家都会拒绝。SQLite 没有 TRUNCATE,
//! 用不带条件的 DELETE(SQLite 会走快速清空)再把 `sqlite_sequence` 里的计数删掉。

pub mod maintenance;
mod sqlite_copy;

use serde::{Deserialize, Serialize};

use crate::design::DdlStatement;
use crate::driver::table_not_found;
use crate::error::{CoreError, CoreResult};
use crate::session::Session;
use crate::sql::bind::quote_string;
use crate::sql::{Dialect, quote_ident, table_name};
use crate::table::TableRef;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyOutcome {
    pub table: TableRef,
    /// 复制了多少行;只复制结构时为 `None`。
    pub rows: Option<u64>,
}

/// 清空表的两种做法。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClearMode {
    /// `DELETE FROM 表`。
    Delete,
    /// `TRUNCATE TABLE 表`;SQLite 是 DELETE 加上自增计数归零。
    Truncate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearOutcome {
    /// 删了多少行;MySQL、PostgreSQL 的 TRUNCATE 不报行数,为 `None`。
    pub rows: Option<u64>,
}

/// `DROP TABLE 表`。
pub fn drop_table_sql(dialect: Dialect, table: &TableRef) -> String {
    format!("DROP TABLE {}", table_name(dialect, table))
}

/// 清空或截断要执行的那一条。PostgreSQL 的截断带上 RESTART IDENTITY,标识列和 serial 的序列一起从头开始。
/// SQLite 没有 TRUNCATE,两种都是 DELETE(截断另外把计数归零,见 [`Session::clear_table`])。
pub fn clear_table_sql(dialect: Dialect, table: &TableRef, mode: ClearMode) -> String {
    let name = table_name(dialect, table);
    match (dialect, mode) {
        (Dialect::Sqlite, _) | (_, ClearMode::Delete) => format!("DELETE FROM {name}"),
        (Dialect::Mysql, ClearMode::Truncate) => format!("TRUNCATE TABLE {name}"),
        (Dialect::Postgres, ClearMode::Truncate) => {
            format!("TRUNCATE TABLE {name} RESTART IDENTITY")
        }
    }
}

/// SQLite:把 AUTOINCREMENT 表在 `sqlite_sequence` 里的计数删掉,下一行从 1 开始。
pub fn reset_sqlite_sequence_sql(table: &TableRef) -> String {
    let d = Dialect::Sqlite;
    format!(
        "DELETE FROM {}.sqlite_sequence WHERE name = {}",
        quote_ident(d, &table.database),
        quote_string(d, &table.table)
    )
}

/// 改名,留在同一个库(schema)里。
pub fn rename_table_sql(dialect: Dialect, table: &TableRef, name: &str) -> String {
    let from = table_name(dialect, table);
    match dialect {
        Dialect::Mysql => {
            let to = TableRef {
                table: name.to_owned(),
                ..table.clone()
            };
            format!("RENAME TABLE {from} TO {}", table_name(dialect, &to))
        }
        Dialect::Postgres | Dialect::Sqlite => {
            format!(
                "ALTER TABLE {from} RENAME TO {}",
                quote_ident(dialect, name)
            )
        }
    }
}

/// 把 `columns` 从旧表搬到新表。PostgreSQL 的标识列要 OVERRIDING SYSTEM VALUE 才写得进去。
pub fn copy_rows_sql(
    dialect: Dialect,
    source: &TableRef,
    target: &TableRef,
    columns: &[String],
) -> String {
    let list: Vec<String> = columns.iter().map(|c| quote_ident(dialect, c)).collect();
    let list = list.join(", ");
    let overriding = if dialect == Dialect::Postgres {
        " OVERRIDING SYSTEM VALUE"
    } else {
        ""
    };
    format!(
        "INSERT INTO {} ({list}){overriding} SELECT {list} FROM {}",
        table_name(dialect, target),
        table_name(dialect, source)
    )
}

/// PostgreSQL:把新表标识列的序列推到已复制数据的最大值,之后插入的行不会撞主键。表是空的时什么都不做。
pub fn reset_identity_sql(target: &TableRef, column: &str) -> String {
    let d = Dialect::Postgres;
    let table = table_name(d, target);
    format!(
        "SELECT setval(pg_get_serial_sequence({}, {}), max({})) FROM {table}",
        quote_string(d, &table),
        quote_string(d, column),
        quote_ident(d, column)
    )
}

fn check_name(source: &TableRef, name: &str) -> CoreResult<TableRef> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidData("新表名不能为空。".into()));
    }
    if name == source.table {
        return Err(CoreError::InvalidData("新表名不能和原表一样。".into()));
    }
    Ok(TableRef {
        table: name.to_owned(),
        ..source.clone()
    })
}

impl Session {
    /// 表名不能为空,PostgreSQL 还要有 schema。
    pub(crate) fn check_table(&self, table: &TableRef) -> CoreResult<()> {
        if table.table.is_empty() {
            return Err(table_not_found(""));
        }
        if matches!(self, Self::Postgres(_)) && table.schema.is_none() {
            return Err(CoreError::Unsupported("PostgreSQL 需要指定 schema".into()));
        }
        Ok(())
    }

    /// 执行改动语句,返回每条影响的行数。MySQL 逐条执行(DDL 会隐式提交),另两家在一个事务里。
    async fn execute_changes(
        &self,
        table: &TableRef,
        statements: &[String],
    ) -> CoreResult<Vec<u64>> {
        match self {
            Self::Mysql(s) => s.execute_each(statements).await.map_err(|(_, e)| e),
            Self::Postgres(s) => s.execute_in_transaction(&table.database, statements).await,
            Self::Sqlite(s) => s.execute_in_transaction(statements).await,
        }
    }

    pub async fn drop_table(&self, table: &TableRef) -> CoreResult<()> {
        self.check_table(table)?;
        let statement = [DdlStatement::execute(drop_table_sql(self.dialect(), table))];
        match self {
            Self::Mysql(s) => s.execute_ddl(&statement).await?,
            Self::Postgres(s) => s.execute_ddl(&table.database, &statement).await?,
            Self::Sqlite(s) => s.execute_ddl(&statement, false).await?,
        };
        Ok(())
    }

    /// 删掉全部行,表结构不动。
    pub async fn clear_table(&self, table: &TableRef, mode: ClearMode) -> CoreResult<ClearOutcome> {
        self.check_table(table)?;
        let dialect = self.dialect();
        let mut statements = vec![clear_table_sql(dialect, table, mode)];
        if let (Self::Sqlite(s), ClearMode::Truncate) = (self, mode)
            && s.has_sqlite_sequence(&table.database).await?
        {
            statements.push(reset_sqlite_sequence_sql(table));
        }
        let counts = self.execute_changes(table, &statements).await?;
        let reports_rows = mode == ClearMode::Delete || dialect == Dialect::Sqlite;
        Ok(ClearOutcome {
            rows: reports_rows.then(|| counts[0]),
        })
    }

    /// 改名为 `name`,返回新表。
    pub async fn rename_table(&self, table: &TableRef, name: &str) -> CoreResult<TableRef> {
        self.check_table(table)?;
        let target = check_name(table, name)?;
        let sql = rename_table_sql(self.dialect(), table, &target.table);
        self.execute_changes(table, &[sql]).await?;
        Ok(target)
    }

    /// 复制表 `source`,新表叫 `name`,和原表在同一个库(schema)里。`with_data` 为假时只复制结构。
    pub async fn copy_table(
        &self,
        source: &TableRef,
        name: &str,
        with_data: bool,
    ) -> CoreResult<CopyOutcome> {
        self.check_table(source)?;
        let target = check_name(source, name)?;
        let dialect = self.dialect();
        // 生成列由数据库算,写不进去。
        let columns: Vec<String> = self
            .table_columns(source)
            .await?
            .into_iter()
            .filter(|c| !c.is_generated)
            .map(|c| c.name)
            .collect();
        let with_data = with_data && !columns.is_empty();
        let insert = with_data.then(|| copy_rows_sql(dialect, source, &target, &columns));
        let rows = match self {
            Self::Mysql(s) => {
                let create = format!(
                    "CREATE TABLE {} LIKE {}",
                    table_name(dialect, &target),
                    table_name(dialect, source)
                );
                let statements: Vec<String> = std::iter::once(create).chain(insert).collect();
                match s.execute_each(&statements).await {
                    Ok(counts) => counts.get(1).copied(),
                    Err((0, error)) => return Err(error),
                    Err((_, error)) => {
                        let cleanup = s.execute_each(&[drop_table_sql(dialect, &target)]).await;
                        return Err(CoreError::Database(match cleanup {
                            Ok(_) => {
                                format!("复制数据失败，刚建的「{}」已经删掉：{error}", target.table)
                            }
                            Err((_, again)) => format!(
                                "复制数据失败：{error}。刚建的「{}」没能删掉（{again}），请手动删除。",
                                target.table
                            ),
                        }));
                    }
                }
            }
            Self::Postgres(s) => {
                let mut statements = vec![format!(
                    "CREATE TABLE {} (LIKE {} INCLUDING ALL)",
                    table_name(dialect, &target),
                    table_name(dialect, source)
                )];
                if let Some(insert) = insert {
                    statements.push(insert);
                    for column in s.identity_columns(source).await? {
                        statements.push(reset_identity_sql(&target, &column));
                    }
                }
                let counts = s
                    .execute_in_transaction(&source.database, &statements)
                    .await?;
                with_data.then(|| counts[1])
            }
            Self::Sqlite(s) => {
                let (table_sql, indexes) = s.schema_sql(source).await?;
                let qualified = table_name(dialect, &target);
                let mut statements = vec![sqlite_copy::rename_table_sql(&table_sql, &qualified)?];
                for (index, sql) in indexes {
                    let new_index = TableRef {
                        table: sqlite_copy::index_name(&index, &source.table, &target.table),
                        ..source.clone()
                    };
                    statements.push(sqlite_copy::rename_index_sql(
                        &sql,
                        &table_name(dialect, &new_index),
                        &quote_ident(dialect, &target.table),
                    )?);
                }
                statements.extend(insert);
                let counts = s.execute_in_transaction(&statements).await?;
                with_data.then(|| counts.last().copied().unwrap_or(0))
            }
        };
        Ok(CopyOutcome {
            table: target,
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(dialect: Dialect, name: &str) -> TableRef {
        TableRef {
            database: "shop".into(),
            schema: (dialect == Dialect::Postgres).then(|| "public".into()),
            table: name.into(),
        }
    }

    #[test]
    fn statements_follow_each_dialect() {
        let pg = Dialect::Postgres;
        assert_eq!(
            drop_table_sql(pg, &table(pg, "users")),
            "DROP TABLE \"public\".\"users\""
        );
        assert_eq!(
            copy_rows_sql(
                pg,
                &table(pg, "users"),
                &table(pg, "users_copy1"),
                &["id".into(), "名".into()]
            ),
            "INSERT INTO \"public\".\"users_copy1\" (\"id\", \"名\") OVERRIDING SYSTEM VALUE SELECT \"id\", \"名\" FROM \"public\".\"users\""
        );
        let my = Dialect::Mysql;
        assert_eq!(
            copy_rows_sql(my, &table(my, "users"), &table(my, "u2"), &["id".into()]),
            "INSERT INTO `shop`.`u2` (`id`) SELECT `id` FROM `shop`.`users`"
        );
        assert_eq!(
            reset_identity_sql(&table(pg, "Users"), "Id"),
            "SELECT setval(pg_get_serial_sequence('\"public\".\"Users\"', 'Id'), max(\"Id\")) FROM \"public\".\"Users\""
        );
    }

    #[test]
    fn clear_and_rename_statements_follow_each_dialect() {
        let (my, pg, lite) = (Dialect::Mysql, Dialect::Postgres, Dialect::Sqlite);
        let truncate = ClearMode::Truncate;
        assert_eq!(
            clear_table_sql(my, &table(my, "users"), truncate),
            "TRUNCATE TABLE `shop`.`users`"
        );
        assert_eq!(
            clear_table_sql(pg, &table(pg, "users"), truncate),
            "TRUNCATE TABLE \"public\".\"users\" RESTART IDENTITY"
        );
        assert_eq!(
            clear_table_sql(lite, &table(lite, "users"), truncate),
            "DELETE FROM \"shop\".\"users\""
        );
        assert_eq!(
            clear_table_sql(my, &table(my, "users"), ClearMode::Delete),
            "DELETE FROM `shop`.`users`"
        );
        assert_eq!(
            reset_sqlite_sequence_sql(&table(lite, "it's")),
            "DELETE FROM \"shop\".sqlite_sequence WHERE name = 'it''s'"
        );
        assert_eq!(
            rename_table_sql(my, &table(my, "users"), "会员"),
            "RENAME TABLE `shop`.`users` TO `shop`.`会员`"
        );
        assert_eq!(
            rename_table_sql(pg, &table(pg, "users"), "Members"),
            "ALTER TABLE \"public\".\"users\" RENAME TO \"Members\""
        );
        assert_eq!(
            rename_table_sql(lite, &table(lite, "users"), "a\"b"),
            "ALTER TABLE \"shop\".\"users\" RENAME TO \"a\"\"b\""
        );
        assert_eq!(
            serde_json::from_value::<ClearMode>(serde_json::json!("truncate")).unwrap(),
            ClearMode::Truncate
        );
    }

    #[test]
    fn new_names_are_checked() {
        let users = table(Dialect::Mysql, "users");
        assert!(
            check_name(&users, "  ")
                .unwrap_err()
                .to_string()
                .contains("不能为空")
        );
        assert!(
            check_name(&users, "users")
                .unwrap_err()
                .to_string()
                .contains("不能和原表一样")
        );
        assert_eq!(
            check_name(&users, " users_copy1 ").unwrap().table,
            "users_copy1"
        );
    }
}
