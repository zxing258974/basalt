//! MySQL / MariaDB 的表结构读取与 DDL 执行。

use sqlx::Row;
use sqlx::mysql::MySqlRow;

use super::mysql::MysqlSession;
use super::table_not_found;
use crate::design::{
    COLUMN_ID_PREFIX, ColumnDef, DdlStatement, FOREIGN_KEY_ID_PREFIX, FkAction, ForeignKeyDef,
    Generated, INDEX_ID_PREFIX, IndexColumn, IndexDef, TableDesign, TableOptions, existing_id,
    string_literal,
};
use crate::error::{CoreError, CoreResult};
use crate::sql::Dialect;
use crate::table::TableRef;

// information_schema 的列在 MySQL 8 里可能是二进制排序规则,统一 CAST 成字符。
const COLUMNS_SQL: &str = "
    SELECT CAST(COLUMN_NAME AS CHAR) AS name,
           CAST(COLUMN_TYPE AS CHAR) AS column_type,
           CAST(IS_NULLABLE AS CHAR) AS is_nullable,
           CAST(COLUMN_DEFAULT AS CHAR) AS column_default,
           CAST(EXTRA AS CHAR) AS extra,
           CAST(COLUMN_COMMENT AS CHAR) AS comment,
           CAST(COLLATION_NAME AS CHAR) AS collation,
           CAST(GENERATION_EXPRESSION AS CHAR) AS generation
    FROM information_schema.COLUMNS
    WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
    ORDER BY ORDINAL_POSITION";

const INDEXES_SQL: &str = "
    SELECT CAST(INDEX_NAME AS CHAR) AS name,
           CAST(NON_UNIQUE AS SIGNED) AS non_unique,
           CAST(COLUMN_NAME AS CHAR) AS column_name,
           CAST(SUB_PART AS SIGNED) AS sub_part,
           CAST(COLLATION AS CHAR) AS collation,
           CAST(INDEX_TYPE AS CHAR) AS index_type
    FROM information_schema.STATISTICS
    WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
    ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX";

const FOREIGN_KEYS_SQL: &str = "
    SELECT CAST(k.CONSTRAINT_NAME AS CHAR) AS name,
           CAST(k.COLUMN_NAME AS CHAR) AS column_name,
           CAST(k.REFERENCED_TABLE_SCHEMA AS CHAR) AS ref_schema,
           CAST(k.REFERENCED_TABLE_NAME AS CHAR) AS ref_table,
           CAST(k.REFERENCED_COLUMN_NAME AS CHAR) AS ref_column,
           CAST(r.UPDATE_RULE AS CHAR) AS on_update,
           CAST(r.DELETE_RULE AS CHAR) AS on_delete
    FROM information_schema.KEY_COLUMN_USAGE k
    JOIN information_schema.REFERENTIAL_CONSTRAINTS r
      ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA
     AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME
     AND r.TABLE_NAME = k.TABLE_NAME
    WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL
    ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION";

const TABLE_SQL: &str = "
    SELECT CAST(ENGINE AS CHAR) AS engine,
           CAST(TABLE_COLLATION AS CHAR) AS collation,
           CAST(TABLE_COMMENT AS CHAR) AS comment
    FROM information_schema.TABLES
    WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND TABLE_TYPE = 'BASE TABLE'";

impl MysqlSession {
    pub async fn read_design(&self, table: &TableRef) -> CoreResult<TableDesign> {
        let fetch = |sql: &'static str| {
            sqlx::query(sql)
                .bind(&table.database)
                .bind(&table.table)
                .fetch_all(&self.pool)
        };
        let Some(table_row) = fetch(TABLE_SQL).await?.into_iter().next() else {
            return Err(table_not_found(&format!(
                "{}.{}",
                table.database, table.table
            )));
        };
        let flavor = Flavor {
            mariadb: self.server().product == "MariaDB",
            backslash_escapes: self.backslash_escapes().await?,
        };
        let mut columns = fetch(COLUMNS_SQL)
            .await?
            .iter()
            .map(|row| column(row, flavor))
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        if flavor.mariadb {
            self.restore_json_type(table, &mut columns).await;
        }
        let (primary_key, indexes) = indexes(&fetch(INDEXES_SQL).await?)?;
        let foreign_keys = foreign_keys(&table.database, &fetch(FOREIGN_KEYS_SQL).await?)?;
        let collation: Option<String> = table_row.try_get("collation")?;
        Ok(TableDesign {
            version: String::new(),
            table: table.clone(),
            columns,
            primary_key,
            primary_key_name: None,
            indexes,
            foreign_keys,
            checks: Vec::new(),
            triggers: Vec::new(),
            options: TableOptions {
                engine: table_row.try_get("engine")?,
                charset: collation.as_deref().map(charset_of),
                collation,
                comment: super::non_empty(table_row.try_get("comment")?),
                ..TableOptions::default()
            },
        })
    }

    /// MariaDB 的 JSON 是 LONGTEXT 加一条字段级的 `json_valid` 检查。按 longtext 写回 MODIFY
    /// 会把检查丢掉,所以认出来后类型写成 json。读不了(10.5.10 以前没有 LEVEL 列)就保持原样。
    async fn restore_json_type(&self, table: &TableRef, columns: &mut [ColumnDef]) {
        let checks: Result<Vec<(String, String)>, _> = sqlx::query_as(
            "SELECT CAST(CONSTRAINT_NAME AS CHAR), CAST(CHECK_CLAUSE AS CHAR)
             FROM information_schema.CHECK_CONSTRAINTS
             WHERE CONSTRAINT_SCHEMA = ? AND TABLE_NAME = ? AND LEVEL = 'Column'",
        )
        .bind(&table.database)
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await;
        let Ok(checks) = checks else { return };
        for column in columns.iter_mut() {
            let json = format!("json_valid(`{}`)", column.name.replace('`', "``"));
            let is_json = checks
                .iter()
                .any(|(name, clause)| *name == column.name && *clause == json);
            if is_json && column.data_type.eq_ignore_ascii_case("longtext") {
                column.data_type = "json".into();
            }
        }
    }

    /// 当前会话里字符串的反斜杠是不是转义符。
    pub async fn backslash_escapes(&self) -> CoreResult<bool> {
        let mode: String = sqlx::query_scalar("SELECT CAST(@@SESSION.sql_mode AS CHAR)")
            .fetch_one(&self.pool)
            .await?;
        Ok(!mode.to_ascii_uppercase().contains("NO_BACKSLASH_ESCAPES"))
    }

    /// 逐条执行。DDL 会隐式提交,中途失败时如实报告哪些已经生效。
    pub async fn execute_ddl(&self, statements: &[DdlStatement]) -> CoreResult<usize> {
        let mut connection = self.pool.acquire().await?;
        for (index, statement) in statements.iter().enumerate() {
            let done = sqlx::query(&statement.sql)
                .persistent(false)
                .execute(&mut *connection)
                .await;
            if let Err(error) = done {
                return Err(partial_failure(index, statements.len(), error.into()));
            }
        }
        Ok(statements.len())
    }
}

#[derive(Clone, Copy)]
struct Flavor {
    mariadb: bool,
    backslash_escapes: bool,
}

fn column(row: &MySqlRow, flavor: Flavor) -> Result<ColumnDef, sqlx::Error> {
    let name: String = row.try_get("name")?;
    let data_type: String = row.try_get("column_type")?;
    let extra = row
        .try_get::<Option<String>, _>("extra")?
        .unwrap_or_default();
    let generation = super::non_empty(row.try_get("generation")?);
    let lower = extra.to_ascii_lowercase();
    let generated = generation
        .filter(|_| lower.contains("generated"))
        .map(|expression| Generated {
            // MySQL 8 的 GENERATION_EXPRESSION 把引号写成 \',原样拼回去是语法错误。
            expression: if flavor.mariadb {
                expression
            } else {
                expression.replace("\\'", "'")
            },
            stored: lower.contains("stored") || lower.contains("persistent"),
        });
    let default = match &generated {
        Some(_) => None,
        None => column_default(row.try_get("column_default")?, &data_type, &lower, flavor),
    };
    Ok(ColumnDef {
        id: existing_id(COLUMN_ID_PREFIX, &name),
        name,
        nullable: row.try_get::<String, _>("is_nullable")? == "YES",
        default,
        auto_increment: lower.contains("auto_increment"),
        comment: super::non_empty(row.try_get("comment")?),
        collation: row.try_get("collation")?,
        on_update: on_update(&extra),
        generated,
        data_type,
    })
}

/// information_schema 里的 COLUMN_DEFAULT 换成能写回 DDL 的表达式。
/// MariaDB 本来就是表达式(字符串带引号,没有默认值的可空列是字面量 NULL);
/// MySQL 给的是值本身,字符串要自己加引号,8.0 的表达式默认值带 DEFAULT_GENERATED 标记。
fn column_default(
    raw: Option<String>,
    data_type: &str,
    extra: &str,
    flavor: Flavor,
) -> Option<String> {
    let raw = raw?;
    if flavor.mariadb {
        return (!raw.eq_ignore_ascii_case("NULL")).then_some(raw);
    }
    if is_current_timestamp(&raw) || raw.starts_with("b'") {
        return Some(raw);
    }
    if extra.contains("default_generated") {
        // 和 GENERATION_EXPRESSION 一样,表达式里的引号被写成了 \'。
        return Some(format!("({})", raw.replace("\\'", "'")));
    }
    if is_numeric_type(data_type) {
        return Some(raw);
    }
    Some(string_literal(
        Dialect::Mysql,
        flavor.backslash_escapes,
        &raw,
    ))
}

fn is_current_timestamp(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let head = lower.split('(').next().unwrap_or_default();
    let args_ok = lower[head.len()..].is_empty()
        || lower[head.len()..]
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|digits| digits.chars().all(|c| c.is_ascii_digit()));
    matches!(
        head,
        "current_timestamp" | "now" | "localtime" | "localtimestamp"
    ) && args_ok
}

fn is_numeric_type(data_type: &str) -> bool {
    let head: String = data_type
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        head.as_str(),
        "tinyint"
            | "smallint"
            | "mediumint"
            | "int"
            | "integer"
            | "bigint"
            | "decimal"
            | "numeric"
            | "float"
            | "double"
            | "real"
            | "bit"
            | "year"
    )
}

/// EXTRA 里的 `on update CURRENT_TIMESTAMP`(MySQL 8 前面还有 DEFAULT_GENERATED)。
fn on_update(extra: &str) -> Option<String> {
    let at = extra.to_ascii_lowercase().find("on update ")?;
    super::non_empty(Some(extra[at + "on update ".len()..].trim().to_owned()))
}

/// 排序规则名以字符集名开头,如 utf8mb4_0900_ai_ci。
fn charset_of(collation: &str) -> String {
    collation.split('_').next().unwrap_or(collation).to_owned()
}

type Keys = (Vec<String>, Vec<IndexDef>);

fn indexes(rows: &[MySqlRow]) -> Result<Keys, sqlx::Error> {
    let mut primary_key = Vec::new();
    let mut indexes: Vec<IndexDef> = Vec::new();
    for row in rows {
        let name: String = row.try_get("name")?;
        let column: Option<String> = row.try_get("column_name")?;
        if name == "PRIMARY" {
            primary_key.extend(column.map(|c| existing_id(COLUMN_ID_PREFIX, &c)));
            continue;
        }
        if indexes.last().is_none_or(|i| i.name != name) {
            let index_type: String = row.try_get("index_type")?;
            let method = Some(index_type.to_ascii_uppercase()).filter(|t| t != "BTREE");
            indexes.push(IndexDef {
                id: existing_id(INDEX_ID_PREFIX, &name),
                unique: row.try_get::<i64, _>("non_unique")? == 0,
                name,
                columns: Vec::new(),
                method,
                definition: None,
                is_constraint: false,
            });
        }
        let index = indexes.last_mut().expect("刚刚放进去");
        match column {
            Some(column) => index.columns.push(IndexColumn {
                column: existing_id(COLUMN_ID_PREFIX, &column),
                descending: row.try_get::<Option<String>, _>("collation")?.as_deref() == Some("D"),
                length: row
                    .try_get::<Option<i64>, _>("sub_part")?
                    .and_then(|n| u32::try_from(n).ok()),
            }),
            // MySQL 8 的函数索引:字段名为空。模型表达不了,只能保留或删除。
            None => index.definition = Some("（函数索引）".into()),
        }
    }
    for index in indexes.iter_mut().filter(|i| i.definition.is_some()) {
        index.columns.clear();
    }
    Ok((primary_key, indexes))
}

fn foreign_keys(database: &str, rows: &[MySqlRow]) -> Result<Vec<ForeignKeyDef>, sqlx::Error> {
    let mut keys: Vec<ForeignKeyDef> = Vec::new();
    for row in rows {
        let name: String = row.try_get("name")?;
        if keys.last().is_none_or(|k| k.name != name) {
            let ref_schema: String = row.try_get("ref_schema")?;
            keys.push(ForeignKeyDef {
                id: existing_id(FOREIGN_KEY_ID_PREFIX, &name),
                name,
                columns: Vec::new(),
                ref_schema: (ref_schema != database).then_some(ref_schema),
                ref_table: row.try_get("ref_table")?,
                ref_columns: Vec::new(),
                on_update: FkAction::parse(&row.try_get::<String, _>("on_update")?),
                on_delete: FkAction::parse(&row.try_get::<String, _>("on_delete")?),
            });
        }
        let key = keys.last_mut().expect("刚刚放进去");
        key.columns.push(existing_id(
            COLUMN_ID_PREFIX,
            &row.try_get::<String, _>("column_name")?,
        ));
        key.ref_columns.push(row.try_get("ref_column")?);
    }
    Ok(keys)
}

/// 第 `index` 条(从 0 起)失败时的报告:前面的已经生效、无法回滚,后面的没有执行。
/// 数据库明确报错的那条没有生效;连接断了、超时这类错误,那一条到底生没生效说不准。
pub(crate) fn partial_failure(index: usize, total: usize, error: CoreError) -> CoreError {
    let n = index + 1;
    let certain = matches!(error, CoreError::Database(_) | CoreError::Authentication(_));
    let skipped = match total - n {
        0 => String::new(),
        1 => format!("第 {} 条没有执行。", n + 1),
        _ => format!("第 {}–{total} 条没有执行。", n + 1),
    };
    if index == 0 && certain {
        let rest = if total > 1 {
            "，后面的语句也没有执行"
        } else {
            ""
        };
        return CoreError::Database(format!(
            "第 1 条语句执行失败，没有任何改动生效{rest}：{error}"
        ));
    }
    let applied = match n {
        1 => "没有。".to_owned(),
        2 => "第 1 条（MySQL 的 DDL 会立即提交，无法回滚）。".to_owned(),
        _ => format!("第 1–{} 条（MySQL 的 DDL 会立即提交，无法回滚）。", n - 1),
    };
    let failed = if certain {
        format!("第 {n} 条语句执行失败：{error}\n已经生效：{applied}没有生效：第 {n} 条。")
    } else {
        format!(
            "第 {n} 条语句执行时连接出了问题：{error}\n已经生效：{applied}第 {n} 条无法确定是否生效。"
        )
    };
    CoreError::PartiallyApplied(format!("{failed}{skipped}请重新读取表结构，核对后再修改。"))
}

#[cfg(test)]
#[path = "mysql_design_tests.rs"]
mod tests;
