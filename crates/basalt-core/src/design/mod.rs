//! 表结构设计:从库里读出的结构、用户在设计器里改过的结构,以及两者之差生成的 DDL。
//!
//! 字段、索引、外键都带一个稳定 id:读出来的用 `col:原名` 这类形式,前端新加的用别的前缀。
//! 生成 DDL 时按 id 配对,所以「改名」能认成改名,不会变成删掉再加。
//! 生成是纯函数(见 [`plan`]),执行与读取在各驱动里。

mod diff;
mod mysql;
mod postgres;
mod render;
mod session;
mod sqlite;
pub(crate) mod sqlite_parse;
mod sqlite_rebuild;
mod validate;

#[cfg(test)]
mod mysql_tests;
#[cfg(test)]
mod postgres_tests;
#[cfg(test)]
mod serde_tests;
#[cfg(test)]
mod sqlite_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod validate_tests;

use serde::{Deserialize, Serialize};

use crate::error::CoreResult;
use crate::sql::Dialect;
use crate::table::TableRef;

/// 读出来的字段 id 前缀;id 带这些前缀却在库里找不到,说明表结构在打开设计器之后变了。
pub const COLUMN_ID_PREFIX: &str = "col:";
pub const INDEX_ID_PREFIX: &str = "idx:";
pub const FOREIGN_KEY_ID_PREFIX: &str = "fk:";
pub const CHECK_ID_PREFIX: &str = "ck:";

pub(crate) fn existing_id(prefix: &str, key: &str) -> String {
    format!("{prefix}{key}")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDesign {
    /// 表所在的库(与 schema)和表名;新建时表名是用户填的,改名时是新名字。
    pub table: TableRef,
    pub columns: Vec<ColumnDef>,
    /// 主键里的字段 id,按主键顺序。
    #[serde(default)]
    pub primary_key: Vec<String>,
    /// 主键约束名:PostgreSQL、SQLite 可能有,MySQL 永远是 PRIMARY。
    #[serde(default)]
    pub primary_key_name: Option<String>,
    #[serde(default)]
    pub indexes: Vec<IndexDef>,
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKeyDef>,
    /// 检查约束,只读:设计器不改它们,SQLite 重建表时原样带上。
    #[serde(default)]
    pub checks: Vec<CheckDef>,
    /// SQLite 表上的触发器,只读:重建表时原样重建。
    #[serde(default)]
    pub triggers: Vec<TriggerDef>,
    #[serde(default)]
    pub options: TableOptions,
    /// 读出来时的结构指纹。草稿从读出的结构复制而来,带着同一个指纹;
    /// 生成 DDL 前重新读一遍,指纹变了说明这期间表被别人改过,草稿里没有的东西不能当成用户删的。
    #[serde(default)]
    pub version: String,
}

impl TableDesign {
    /// 结构指纹:除指纹本身以外全部内容的哈希。
    pub fn fingerprint(&self) -> String {
        use std::hash::{Hash, Hasher};
        let unversioned = TableDesign {
            version: String::new(),
            ..self.clone()
        };
        let text = serde_json::to_string(&unversioned).unwrap_or_default();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDef {
    pub id: String,
    pub name: String,
    /// 完整类型,如 `varchar(50)`、`int unsigned`、`numeric(10,2)`;SQLite 可以为空。
    pub data_type: String,
    pub nullable: bool,
    /// 默认值的 SQL 表达式原文,如 `'abc'`、`0`、`CURRENT_TIMESTAMP`。
    #[serde(default)]
    pub default: Option<String>,
    /// MySQL 的 AUTO_INCREMENT、PostgreSQL 的标识列、SQLite 的 AUTOINCREMENT。
    #[serde(default)]
    pub auto_increment: bool,
    #[serde(default)]
    pub comment: Option<String>,
    /// 排序规则。设计器不改它,改字段时原样带上,免得 MySQL 的 MODIFY 把它丢掉。
    #[serde(default)]
    pub collation: Option<String>,
    /// MySQL 的 ON UPDATE 子句,如 `CURRENT_TIMESTAMP`。
    #[serde(default)]
    pub on_update: Option<String>,
    /// 生成列;有它就不能有默认值,也不能自增。
    #[serde(default)]
    pub generated: Option<Generated>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    pub expression: String,
    /// 存储列(STORED);否则是虚拟列(VIRTUAL)。
    pub stored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDef {
    pub id: String,
    pub name: String,
    pub columns: Vec<IndexColumn>,
    #[serde(default)]
    pub unique: bool,
    /// MySQL 的 FULLTEXT / SPATIAL / HASH,PostgreSQL 的 gin、gist……;`None` 是默认的 B 树。
    #[serde(default)]
    pub method: Option<String>,
    /// 模型表达不了的索引(表达式、部分索引……)的原始定义:只能删除,不能修改。
    #[serde(default)]
    pub definition: Option<String>,
    /// 唯一约束带出来的索引(PostgreSQL 的 UNIQUE 约束、SQLite 的 UNIQUE 列约束)。
    /// 只认库里读出来的值,不信前端传回来的。
    #[serde(default)]
    pub is_constraint: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexColumn {
    /// 字段 id。
    pub column: String,
    #[serde(default)]
    pub descending: bool,
    /// MySQL 的前缀长度,如 `name(10)`。
    #[serde(default)]
    pub length: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyDef {
    pub id: String,
    /// 约束名;为空时由数据库自动命名(SQLite 则不命名)。
    pub name: String,
    /// 本表的字段 id。
    pub columns: Vec<String>,
    /// 被引用表所在的库(MySQL)或 schema(PostgreSQL);`None` 表示和本表相同。
    #[serde(default)]
    pub ref_schema: Option<String>,
    pub ref_table: String,
    /// 被引用的字段名;PostgreSQL、SQLite 可以为空,表示引用对方的主键。
    pub ref_columns: Vec<String>,
    #[serde(default)]
    pub on_update: FkAction,
    #[serde(default)]
    pub on_delete: FkAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FkAction {
    #[default]
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl FkAction {
    pub fn sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }

    /// 解析 information_schema、PRAGMA 给出的规则名;认不出的当作 NO ACTION。
    pub fn parse(rule: &str) -> Self {
        match rule.trim().to_ascii_uppercase().as_str() {
            "RESTRICT" => Self::Restrict,
            "CASCADE" => Self::Cascade,
            "SET NULL" => Self::SetNull,
            "SET DEFAULT" => Self::SetDefault,
            _ => Self::NoAction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckDef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub expression: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDef {
    pub name: String,
    pub sql: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOptions {
    /// MySQL 的存储引擎。
    #[serde(default)]
    pub engine: Option<String>,
    /// MySQL 的默认字符集与排序规则。
    #[serde(default)]
    pub charset: Option<String>,
    #[serde(default)]
    pub collation: Option<String>,
    /// 表注释(MySQL、PostgreSQL)。
    #[serde(default)]
    pub comment: Option<String>,
    /// SQLite 的 WITHOUT ROWID 与 STRICT,只读,重建表时保留。
    #[serde(default)]
    pub without_rowid: bool,
    #[serde(default)]
    pub strict: bool,
}

/// 语句怎么执行。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecMode {
    /// 放在一个事务里,失败整体回滚(PostgreSQL、SQLite)。
    Transaction,
    /// MySQL:DDL 会隐式提交,只能逐条执行。
    Autocommit,
    /// SQLite 重建表:临时关掉外键约束,在一个事务里执行,提交前做外键检查。
    Rebuild,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatementKind {
    Execute,
    /// `PRAGMA foreign_key_check`:返回任何一行都算失败。
    ForeignKeyCheck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DdlStatement {
    pub sql: String,
    pub kind: StatementKind,
}

impl DdlStatement {
    pub fn execute(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            kind: StatementKind::Execute,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DdlPlan {
    pub mode: ExecMode,
    pub statements: Vec<DdlStatement>,
}

impl DdlPlan {
    pub fn preview(&self) -> DdlPreview {
        DdlPreview {
            mode: self.mode,
            statements: self.statements.iter().map(|s| s.sql.clone()).collect(),
        }
    }
}

/// 给用户确认的 SQL。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DdlPreview {
    pub statements: Vec<String>,
    pub mode: ExecMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DdlOutcome {
    pub statements: usize,
    /// 执行后的表(改名后是新名字)。
    pub table: TableRef,
}

/// SQLite 库里别的对象,重建表前用来检查有没有东西引用了要改名、删除的字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaObject {
    /// table / view / trigger / index
    pub kind: String,
    pub name: String,
    pub sql: String,
}

/// 生成 DDL 时需要的、表结构以外的信息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DdlContext {
    pub dialect: Dialect,
    /// MySQL 的字符串里反斜杠是不是转义符(sql_mode 没有 NO_BACKSLASH_ESCAPES)。
    pub backslash_escapes: bool,
    /// SQLite:库里其它对象的定义,见 [`SchemaObject`]。
    pub schema_objects: Vec<SchemaObject>,
    /// SQLite:有外键引用这张表的其它表,重建后要一起做外键检查。
    pub referencing_tables: Vec<String>,
    /// SQLite:表定义里重建会丢掉的写法,有就不能重建。
    pub rebuild_blockers: Vec<String>,
}

impl DdlContext {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            backslash_escapes: true,
            schema_objects: Vec::new(),
            referencing_tables: Vec::new(),
            rebuild_blockers: Vec::new(),
        }
    }
}

/// PostgreSQL 的建表语句(建表、建索引、注释),给信息面板看:PostgreSQL 没有 SHOW CREATE TABLE。
/// 只是展示,不做设计器的校验。
pub fn postgres_create_statements(design: &TableDesign) -> Vec<String> {
    postgres::create_table(design)
}

/// 由「原结构 + 新结构」生成要执行的语句。`original` 为 `None` 表示新建表。
pub fn plan(
    context: &DdlContext,
    original: Option<&TableDesign>,
    target: &TableDesign,
) -> CoreResult<DdlPlan> {
    validate::validate(context.dialect, original, target)?;
    match context.dialect {
        Dialect::Mysql => mysql::plan(context, original, target),
        Dialect::Postgres => postgres::plan(original, target),
        Dialect::Sqlite => sqlite::plan(context, original, target),
    }
}

/// 按方言转义的字符串字面量。MySQL 默认把反斜杠当转义符。
pub(crate) fn string_literal(dialect: Dialect, backslash_escapes: bool, text: &str) -> String {
    let escaped = text.replace('\'', "''");
    let escaped = if dialect == Dialect::Mysql && backslash_escapes {
        escaped.replace('\\', "\\\\")
    } else {
        escaped
    };
    format!("'{escaped}'")
}
