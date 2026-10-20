//! 表数据浏览与编辑的请求、响应结构。SQL 生成见 [`crate::sql`],执行见各驱动。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::value::CellValue;

/// 一页最多读多少行。再多前端也只是滚不完,还会拖慢解码。
pub const MAX_PAGE_SIZE: u32 = 5000;

/// 要读写的表。PostgreSQL 必须带 schema;MySQL、SQLite 的 schema 为空。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRef {
    pub database: String,
    #[serde(default)]
    pub schema: Option<String>,
    pub table: String,
}

/// 写回时怎么把用户输入的文本变成参数。由驱动按列的声明类型推出。
/// 也发给前端:二进制列才接受 0x 十六进制输入。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ValueType {
    Integer {
        unsigned: bool,
    },
    Float,
    Decimal,
    /// MySQL 的 BIT(n):按无符号整数写入。
    Bit,
    /// 输入以 0x 开头时按十六进制解析成字节,否则按文本写入。
    Binary,
    /// 其余一律按文本写入,由数据库自己转换。
    #[default]
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    /// 给人看的完整类型,如 `varchar(50)`、`int unsigned`、`character varying(50)`。
    pub data_type: String,
    pub is_primary_key: bool,
    pub nullable: bool,
    /// 有默认值、自增或标识列,新增行时可以不填。
    pub has_default: bool,
    /// 生成列,只能看不能写。
    pub is_generated: bool,
    pub value_type: ValueType,
    /// PostgreSQL:驱动解不了的类型在 SELECT 里转成 text 再读。
    #[serde(skip)]
    pub select_as_text: bool,
    /// PostgreSQL:写回时参数要转换成的类型,`None` 表示文本参数可以直接用。
    #[serde(skip)]
    pub write_cast: Option<String>,
}

impl ColumnInfo {
    /// 测试与驱动共用的构造入口,其余字段按需覆盖。
    pub fn new(name: &str, data_type: &str) -> Self {
        Self {
            name: name.to_owned(),
            data_type: data_type.to_owned(),
            is_primary_key: false,
            nullable: true,
            has_default: false,
            is_generated: false,
            value_type: ValueType::Text,
            select_as_text: false,
            write_cast: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortSpec {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub offset: u64,
    pub limit: u32,
    #[serde(default)]
    pub sort: Option<SortSpec>,
    /// 用户在筛选框里写的 WHERE 条件(不含 WHERE 关键字)。
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePage {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<CellValue>>,
    /// 后面还有没有行。多读一行来判断,不做 COUNT(*)。
    pub has_more: bool,
    /// 不能编辑时的原因,能编辑时为 `None`。
    pub read_only_reason: Option<String>,
    /// 实际执行的 SELECT,显示在页脚。
    pub sql: String,
}

/// 用户输入的新值:`None` 是 NULL,其余是编辑框里的原文。
pub type EditValues = BTreeMap<String, Option<String>>;

/// 主键列 → 读出来时的原值。
pub type RowKey = BTreeMap<String, CellValue>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowChange {
    Update { key: RowKey, values: EditValues },
    Insert { values: EditValues },
    Delete { key: RowKey },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    pub statements: usize,
    pub affected_rows: u64,
}

/// 没有主键就无法安全地定位一行:按全部列匹配可能命中多行,也可能因为浮点、
/// 时间精度匹配不上。这种表只读。
pub fn read_only_reason(columns: &[ColumnInfo]) -> Option<String> {
    if columns.iter().any(|c| c.is_primary_key) {
        None
    } else {
        Some("这张表没有主键，无法安全地定位要修改的行，只能查看。".into())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn row_changes_use_kind_tag_and_null_for_sql_null() {
        let change: RowChange = serde_json::from_value(json!({
            "kind": "update",
            "key": { "id": { "kind": "int", "value": "7" } },
            "values": { "name": "Bob", "note": null }
        }))
        .unwrap();
        let RowChange::Update { key, values } = change else {
            panic!("应该是 update");
        };
        assert_eq!(key["id"], CellValue::Int(7));
        assert_eq!(values["name"].as_deref(), Some("Bob"));
        assert_eq!(values["note"], None);
    }

    #[test]
    fn internal_column_hints_are_not_serialized_but_value_type_is() {
        let column = ColumnInfo {
            select_as_text: true,
            write_cast: Some("inet".into()),
            ..ColumnInfo::new("addr", "inet")
        };
        assert_eq!(
            serde_json::to_value(&column).unwrap(),
            json!({
                "name": "addr",
                "dataType": "inet",
                "isPrimaryKey": false,
                "nullable": true,
                "hasDefault": false,
                "isGenerated": false,
                "valueType": { "kind": "text" }
            })
        );
    }

    #[test]
    fn integer_value_types_carry_signedness() {
        assert_eq!(
            serde_json::to_value(ValueType::Integer { unsigned: true }).unwrap(),
            json!({ "kind": "integer", "unsigned": true })
        );
        assert_eq!(
            serde_json::to_value(ValueType::Binary).unwrap(),
            json!({ "kind": "binary" })
        );
    }

    #[test]
    fn tables_without_primary_key_are_read_only() {
        let id = ColumnInfo {
            is_primary_key: true,
            ..ColumnInfo::new("id", "int")
        };
        assert_eq!(read_only_reason(std::slice::from_ref(&id)), None);
        assert!(
            read_only_reason(&[ColumnInfo::new("name", "text")])
                .unwrap()
                .contains("没有主键")
        );
    }
}
