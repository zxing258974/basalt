//! 与具体连接无关的 SQL 工具:方言、标识符转义、筛选条件检查、分页 SELECT 与 DML 生成。

pub mod bind;
pub mod dml;
pub mod filter;
pub mod insert;
pub mod select;
#[cfg(test)]
mod test_support;

use serde::{Deserialize, Serialize};

use crate::config::DriverKind;
use crate::error::{CoreError, CoreResult};
use crate::table::TableRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    Mysql,
    Postgres,
    Sqlite,
}

/// Redis 没有 SQL 方言。
impl TryFrom<DriverKind> for Dialect {
    type Error = CoreError;

    fn try_from(driver: DriverKind) -> CoreResult<Self> {
        match driver {
            DriverKind::Mysql | DriverKind::Mariadb => Ok(Self::Mysql),
            DriverKind::Postgres => Ok(Self::Postgres),
            DriverKind::Sqlite => Ok(Self::Sqlite),
            DriverKind::Redis => Err(CoreError::Unsupported("Redis 不是 SQL 数据库".into())),
        }
    }
}

/// 交给 sqlx 执行的运行时 SQL。sqlx 0.9 起只直接认 `&'static str`,运行时拼的语句要显式声明审核过。
///
/// Basalt 是数据库客户端,大部分语句本来就是运行时的:用户在查询标签页里写的原样执行;
/// 自己拼的语句里,标识符经过 [`quote_ident`],值走占位符或经 [`bind::literal`] 转义。
pub(crate) fn dynamic(sql: impl Into<String>) -> sqlx::SqlStr {
    sqlx::SqlSafeStr::into_sql_str(sqlx::AssertSqlSafe(sql.into()))
}

/// 按方言给标识符加引号,内部的引号字符成对转义。
pub fn quote_ident(dialect: Dialect, name: &str) -> String {
    let quote = match dialect {
        Dialect::Mysql => '`',
        Dialect::Postgres | Dialect::Sqlite => '"',
    };
    let escaped = name.replace(quote, &format!("{quote}{quote}"));
    format!("{quote}{escaped}{quote}")
}

/// 用点连接多段限定名,如 `db`.`table`;空段跳过。
pub fn qualified(dialect: Dialect, parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| quote_ident(dialect, p))
        .collect::<Vec<_>>()
        .join(".")
}

/// 表的限定名:MySQL、SQLite 是 库.表,PostgreSQL 是 schema.表(库由连接决定)。
pub fn table_name(dialect: Dialect, table: &TableRef) -> String {
    match dialect {
        Dialect::Postgres => qualified(
            dialect,
            &[table.schema.as_deref().unwrap_or_default(), &table.table],
        ),
        Dialect::Mysql | Dialect::Sqlite => qualified(dialect, &[&table.database, &table.table]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_uses_backticks_and_doubles_embedded_ones() {
        assert_eq!(quote_ident(Dialect::Mysql, "users"), "`users`");
        assert_eq!(quote_ident(Dialect::Mysql, "we`ird"), "`we``ird`");
        assert_eq!(quote_ident(Dialect::Mysql, "has \"dq\""), "`has \"dq\"`");
    }

    #[test]
    fn postgres_and_sqlite_use_double_quotes() {
        assert_eq!(quote_ident(Dialect::Postgres, "Order"), "\"Order\"");
        assert_eq!(quote_ident(Dialect::Sqlite, "a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident(Dialect::Postgres, "用户表"), "\"用户表\"");
    }

    #[test]
    fn qualified_joins_non_empty_parts() {
        assert_eq!(
            qualified(Dialect::Mysql, &["shop", "orders"]),
            "`shop`.`orders`"
        );
        assert_eq!(
            qualified(Dialect::Postgres, &["", "public", "t"]),
            "\"public\".\"t\""
        );
    }

    #[test]
    fn mariadb_maps_to_mysql_dialect() {
        assert_eq!(
            Dialect::try_from(DriverKind::Mariadb).unwrap(),
            Dialect::Mysql
        );
        assert_eq!(
            Dialect::try_from(DriverKind::Sqlite).unwrap(),
            Dialect::Sqlite
        );
        assert!(Dialect::try_from(DriverKind::Redis).is_err());
    }
}
