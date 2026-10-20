//! 与具体连接无关的 SQL 工具:方言、标识符转义。后续的 DML / DDL 生成也放这里。

use serde::{Deserialize, Serialize};

use crate::config::DriverKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    Mysql,
    Postgres,
    Sqlite,
}

impl From<DriverKind> for Dialect {
    fn from(driver: DriverKind) -> Self {
        match driver {
            DriverKind::Mysql | DriverKind::Mariadb => Self::Mysql,
            DriverKind::Postgres => Self::Postgres,
            DriverKind::Sqlite => Self::Sqlite,
        }
    }
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
        assert_eq!(Dialect::from(DriverKind::Mariadb), Dialect::Mysql);
        assert_eq!(Dialect::from(DriverKind::Sqlite), Dialect::Sqlite);
    }
}
