//! 引用这张表的外键(别的表上的):清空表(DELETE)时它们按 ON DELETE 规则连带删掉、置空别的表里的行,
//! 或者在有引用的行时让清空失败;MySQL、PostgreSQL 的截断(TRUNCATE)只要有引用就被拒绝。
//! 确认框据此写明会波及哪些表。自引用(表引用自己)不算:清空时那些行本来就要删。

use serde::Serialize;

use crate::error::CoreResult;
use crate::session::Session;
use crate::table::TableRef;

/// 外键的 ON DELETE 规则。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OnDelete {
    Cascade,
    SetNull,
    SetDefault,
    Restrict,
    NoAction,
}

impl OnDelete {
    /// MySQL、SQLite 写的是 `CASCADE`、`SET NULL`……,PostgreSQL 的 `confdeltype` 是一个字母。
    pub(crate) fn parse(rule: &str) -> Self {
        match rule.trim().to_ascii_uppercase().as_str() {
            "CASCADE" | "C" => Self::Cascade,
            "SET NULL" | "N" => Self::SetNull,
            "SET DEFAULT" | "D" => Self::SetDefault,
            "RESTRICT" | "R" => Self::Restrict,
            _ => Self::NoAction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferencingKey {
    /// 外键所在的表。
    pub table: TableRef,
    /// 约束名;SQLite 的外键没有名字。
    pub name: Option<String>,
    pub on_delete: OnDelete,
}

impl Session {
    /// 别的表上引用这张表的外键。
    pub async fn referencing_keys(&self, table: &TableRef) -> CoreResult<Vec<ReferencingKey>> {
        self.check_table(table)?;
        match self {
            Self::Mysql(s) => s.referencing_keys(table).await,
            Self::Postgres(s) => s.referencing_keys(table).await,
            Self::Sqlite(s) => s.referencing_keys(table).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_read_from_every_spelling() {
        assert_eq!(OnDelete::parse("CASCADE"), OnDelete::Cascade);
        assert_eq!(OnDelete::parse("c"), OnDelete::Cascade);
        assert_eq!(OnDelete::parse("SET NULL"), OnDelete::SetNull);
        assert_eq!(OnDelete::parse("n"), OnDelete::SetNull);
        assert_eq!(OnDelete::parse("SET DEFAULT"), OnDelete::SetDefault);
        assert_eq!(OnDelete::parse("r"), OnDelete::Restrict);
        assert_eq!(OnDelete::parse("NO ACTION"), OnDelete::NoAction);
        assert_eq!(OnDelete::parse("a"), OnDelete::NoAction);
        assert_eq!(
            serde_json::to_value(OnDelete::SetNull).unwrap(),
            serde_json::json!("setNull")
        );
    }
}
