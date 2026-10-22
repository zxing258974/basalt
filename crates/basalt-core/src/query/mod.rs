//! SQL 编辑器:执行用户自己写的 SQL、取消执行、执行历史与保存的查询。
//!
//! 每个查询标签页有一条自己的连接(不从连接池里借,见 [`editor`]):会话变量、临时表、
//! 未提交的事务、USE 切换的库都留在这条连接上,不会漏进表数据浏览用的连接池。
//! 语句由前端切分好、一次发一条,这里只负责执行与把结果解码成 [`CellValue`]。

pub(crate) mod collect;
pub(crate) mod editor;
pub mod history;
pub mod lease;
pub mod results;
pub mod saved;

pub use editor::EditorRegistry;
use serde::{Deserialize, Serialize};

use crate::table::TableRef;
use crate::value::CellValue;

/// 查询标签页所在的库和 schema。MySQL 的库可以不选;schema 只有 PostgreSQL 用。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryScope {
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub schema: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultColumn {
    pub name: String,
    /// 驱动报的类型名(小写),如 varchar、int4;认不出时为空。
    pub data_type: String,
    /// 这一列直接取自哪张表的哪一列;表达式、聚合、驱动说不清的为 `None`。前端据此判断结果能不能改。
    pub source: Option<ColumnSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSource {
    pub table: TableRef,
    /// 表里的列名(结果里可能起了别名)。
    pub column: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSet {
    /// 完整的结果存在后端(见 [`results`]),往下滚、排序时按这个编号去取。
    pub id: u64,
    pub columns: Vec<ResultColumn>,
    /// 前 [`results::FIRST_BATCH`] 行。
    pub rows: Vec<Vec<CellValue>>,
    /// 存下来的总行数。
    pub total_rows: u64,
    /// 结果太大(超出内存预算,见 [`results::MEMORY_BUDGET`]),只存了前 `total_rows` 行。
    pub truncated: bool,
    /// 列的来源看得出、却不能在结果里改的原因,如来自临时表。
    pub read_only: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutcomeStatus {
    Ok,
    Error,
    Cancelled,
}

/// 一条语句的执行结果。数据库报错、被取消也是一个结果(status 说明),
/// 连接没打开这类执行之前的失败才是 `Err`。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementOutcome {
    pub status: OutcomeStatus,
    /// 出错或取消时数据库给的原文。
    pub message: Option<String>,
    /// 一条语句可以有多个结果集(如 MySQL 的 CALL)。出错时是出错之前已经读到的。
    pub result_sets: Vec<ResultSet>,
    /// 增删改的影响行数;查询、DDL 这类没有意义的为 `None`。
    pub affected_rows: Option<u64>,
    pub elapsed_ms: u64,
    /// 执行之后这条连接所在的库 / schema:USE、SET search_path 改了它时前端跟着改。
    pub scope: QueryScope,
    /// 执行前发生的、用户应该知道的事,如连接断开后重新连上了。
    pub notice: Option<String>,
}

/// 语句的第一个关键字(大写),跳过开头的空白、注释和左括号。没有时为空串。
pub fn leading_keyword(sql: &str) -> String {
    let mut rest = sql;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '(');
        if rest.starts_with("--") || rest.starts_with('#') {
            rest = rest.find('\n').map_or("", |at| &rest[at..]);
        } else if rest.starts_with("/*") && !rest.starts_with("/*!") {
            rest = rest.find("*/").map_or("", |at| &rest[at + 2..]);
        } else {
            break;
        }
    }
    rest.chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
        .collect::<String>()
        .to_ascii_uppercase()
}

/// 可能返回结果集的语句:没读到行时,再按语句结构取列名,显示一个空结果。
pub(crate) fn may_return_rows(keyword: &str) -> bool {
    matches!(
        keyword,
        "SELECT"
            | "WITH"
            | "SHOW"
            | "DESCRIBE"
            | "DESC"
            | "EXPLAIN"
            | "VALUES"
            | "TABLE"
            | "PRAGMA"
    )
}

/// 增删改:影响行数有意义。
pub(crate) fn is_dml(keyword: &str) -> bool {
    matches!(
        keyword,
        "INSERT" | "UPDATE" | "DELETE" | "REPLACE" | "MERGE" | "UPSERT" | "COPY"
    )
}

/// 改表结构的语句。
pub(crate) fn is_ddl(keyword: &str) -> bool {
    matches!(keyword, "CREATE" | "ALTER" | "DROP" | "RENAME" | "TRUNCATE")
}

/// 执行后可能换了库或 schema 的语句,执行完要重新读一下。
pub(crate) fn may_change_scope(keyword: &str) -> bool {
    matches!(keyword, "USE" | "SET")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn leading_keyword_skips_whitespace_comments_and_parens() {
        assert_eq!(leading_keyword("  select 1"), "SELECT");
        assert_eq!(
            leading_keyword("-- 说明\n# 另一行\n/* 块 */ Update t"),
            "UPDATE"
        );
        assert_eq!(leading_keyword("((SELECT 1) UNION (SELECT 2))"), "SELECT");
        assert_eq!(leading_keyword("show_tables"), "SHOW_TABLES");
        assert_eq!(leading_keyword("/*!40101 SET NAMES utf8 */"), "");
        assert_eq!(leading_keyword("-- 只有注释"), "");
        assert_eq!(leading_keyword(""), "");
    }

    #[test]
    fn statement_kinds() {
        assert!(may_return_rows("WITH"));
        assert!(may_return_rows("PRAGMA"));
        assert!(!may_return_rows("INSERT"));
        assert!(is_dml("DELETE"));
        assert!(!is_dml("CREATE"));
        assert!(is_ddl("ALTER"));
        assert!(!is_ddl("SELECT"));
        assert!(may_change_scope("USE"));
        assert!(!may_change_scope("SELECT"));
    }

    #[test]
    fn outcome_serializes_in_camel_case() {
        let outcome = StatementOutcome {
            status: OutcomeStatus::Cancelled,
            message: Some("Query execution was interrupted".into()),
            result_sets: vec![ResultSet {
                id: 3,
                columns: vec![ResultColumn {
                    name: "n".into(),
                    data_type: "bigint".into(),
                    source: Some(ColumnSource {
                        table: TableRef {
                            database: "shop".into(),
                            schema: None,
                            table: "orders".into(),
                        },
                        column: "id".into(),
                    }),
                }],
                rows: vec![vec![CellValue::Int(1)]],
                total_rows: 1,
                truncated: false,
                read_only: None,
            }],
            affected_rows: None,
            elapsed_ms: 12,
            scope: QueryScope {
                database: Some("shop".into()),
                schema: None,
            },
            notice: None,
        };
        assert_eq!(
            serde_json::to_value(&outcome).unwrap(),
            json!({
                "status": "cancelled",
                "message": "Query execution was interrupted",
                "resultSets": [{
                    "id": 3,
                    "columns": [{
                        "name": "n",
                        "dataType": "bigint",
                        "source": {
                            "table": { "database": "shop", "schema": null, "table": "orders" },
                            "column": "id"
                        }
                    }],
                    "rows": [[{ "kind": "int", "value": "1" }]],
                    "totalRows": 1,
                    "truncated": false,
                    "readOnly": null
                }],
                "affectedRows": null,
                "elapsedMs": 12,
                "scope": { "database": "shop", "schema": null },
                "notice": null
            })
        );
    }

    #[test]
    fn scope_fields_default_to_none() {
        let scope: QueryScope = serde_json::from_value(json!({})).unwrap();
        assert_eq!(scope, QueryScope::default());
    }
}
