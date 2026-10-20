//! SQLite 的表结构读取(执行见 sqlite_ddl)。PRAGMA 给不了的(排序规则、生成列表达式、检查约束、
//! AUTOINCREMENT)从 CREATE TABLE 原文里解析。

use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::sqlite::SqliteSession;
use super::table_not_found;
use crate::design::sqlite_parse::{ParsedTable, parse_create_table};
use crate::design::{
    CHECK_ID_PREFIX, COLUMN_ID_PREFIX, CheckDef, ColumnDef, FOREIGN_KEY_ID_PREFIX, FkAction,
    ForeignKeyDef, Generated, INDEX_ID_PREFIX, IndexColumn, IndexDef, SchemaObject, TableDesign,
    TableOptions, TriggerDef, existing_id,
};
use crate::error::{CoreError, CoreResult};
use crate::sql::{Dialect, quote_ident};
use crate::table::TableRef;

/// table_xinfo 的 hidden:1 是虚拟表的隐藏列,2、3 是生成列(虚拟、存储)。
const HIDDEN_VIRTUAL_TABLE: i64 = 1;
const HIDDEN_STORED: i64 = 3;

impl SqliteSession {
    pub async fn read_design(&self, table: &TableRef) -> CoreResult<TableDesign> {
        let master = format!(
            "{}.sqlite_master",
            quote_ident(Dialect::Sqlite, &table.database)
        );
        let sql: Option<String> = sqlx::query_scalar(&format!(
            "SELECT sql FROM {master} WHERE type = 'table' AND name = ?1"
        ))
        .bind(&table.table)
        .fetch_optional(&self.pool)
        .await?;
        let Some(sql) = sql else {
            return Err(table_not_found(&table.table));
        };
        let parsed = parse_create_table(&sql).unwrap_or_default();
        let pragma = |name: &str| format!("SELECT * FROM pragma_{name}(?1, ?2)");
        let xinfo = sqlx::query(&pragma("table_xinfo"))
            .bind(&table.table)
            .bind(&table.database)
            .fetch_all(&self.pool)
            .await?;
        let mut keyed: Vec<(i64, String)> = Vec::new();
        let mut columns = Vec::new();
        for row in &xinfo {
            if row.try_get::<i64, _>("hidden")? == HIDDEN_VIRTUAL_TABLE {
                continue;
            }
            let column = column(row, &parsed)?;
            let pk: i64 = row.try_get("pk")?;
            if pk > 0 {
                keyed.push((pk, column.id.clone()));
            }
            columns.push(column);
        }
        keyed.sort();
        let fk_rows = sqlx::query(&pragma("foreign_key_list"))
            .bind(&table.table)
            .bind(&table.database)
            .fetch_all(&self.pool)
            .await?;
        let triggers = sqlx::query(&format!(
            "SELECT name, sql FROM {master} WHERE type = 'trigger' AND tbl_name = ?1 COLLATE NOCASE ORDER BY name"
        ))
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?;
        Ok(TableDesign {
            version: String::new(),
            table: table.clone(),
            indexes: self.indexes(table, &master, &columns).await?,
            columns,
            primary_key: keyed.into_iter().map(|(_, id)| id).collect(),
            primary_key_name: None,
            foreign_keys: foreign_keys(&fk_rows, &parsed)?,
            checks: parsed
                .checks
                .iter()
                .enumerate()
                .map(|(i, (name, expression))| CheckDef {
                    id: existing_id(CHECK_ID_PREFIX, &i.to_string()),
                    name: name.clone(),
                    expression: expression.clone(),
                })
                .collect(),
            triggers: triggers
                .iter()
                .map(|row| {
                    Ok(TriggerDef {
                        name: row.try_get("name")?,
                        sql: row.try_get("sql")?,
                    })
                })
                .collect::<Result<_, sqlx::Error>>()?,
            options: TableOptions {
                without_rowid: parsed.without_rowid,
                strict: parsed.strict,
                ..TableOptions::default()
            },
        })
    }

    async fn indexes(
        &self,
        table: &TableRef,
        master: &str,
        columns: &[ColumnDef],
    ) -> CoreResult<Vec<IndexDef>> {
        let list = sqlx::query(
            "SELECT name, \"unique\", origin, partial FROM pragma_index_list(?1, ?2) ORDER BY name",
        )
        .bind(&table.table)
        .bind(&table.database)
        .fetch_all(&self.pool)
        .await?;
        let mut indexes = Vec::new();
        for row in &list {
            let origin: String = row.try_get("origin")?;
            if origin == "pk" {
                continue;
            }
            let name: String = row.try_get("name")?;
            let parts = sqlx::query(
                "SELECT cid, name, \"desc\", coll FROM pragma_index_xinfo(?1, ?2) WHERE key = 1 ORDER BY seqno",
            )
            .bind(&name)
            .bind(&table.database)
            .fetch_all(&self.pool)
            .await?;
            let mut raw = row.try_get::<i64, _>("partial")? == 1;
            let mut index_columns = Vec::new();
            for part in &parts {
                let column: Option<String> = part.try_get("name")?;
                let collation: Option<String> = part.try_get("coll")?;
                let declared = column
                    .as_deref()
                    .and_then(|c| columns.iter().find(|d| d.name.eq_ignore_ascii_case(c)))
                    .and_then(|d| d.collation.clone())
                    .unwrap_or_else(|| "BINARY".into());
                // 表达式列(cid = -2)、另行指定排序规则的列,模型都表达不了。
                raw |= column.is_none()
                    || !collation
                        .unwrap_or_default()
                        .eq_ignore_ascii_case(&declared);
                if let Some(column) = column {
                    index_columns.push(IndexColumn {
                        column: existing_id(COLUMN_ID_PREFIX, &column),
                        descending: part.try_get::<i64, _>("desc")? == 1,
                        length: None,
                    });
                }
            }
            let definition = if raw && origin == "c" {
                sqlx::query_scalar(&format!(
                    "SELECT sql FROM {master} WHERE type = 'index' AND name = ?1"
                ))
                .bind(&name)
                .fetch_optional(&self.pool)
                .await?
                .flatten()
            } else {
                None
            };
            indexes.push(IndexDef {
                id: existing_id(INDEX_ID_PREFIX, &name),
                name,
                columns: if definition.is_some() {
                    Vec::new()
                } else {
                    index_columns
                },
                unique: row.try_get::<i64, _>("unique")? == 1,
                method: None,
                definition,
                is_constraint: origin == "u",
            });
        }
        Ok(indexes)
    }

    /// 重建表会丢掉的写法(见 [`ParsedTable::unsupported`]),有就不能重建。
    pub async fn rebuild_blockers(&self, table: &TableRef) -> CoreResult<Vec<String>> {
        let master = format!(
            "{}.sqlite_master",
            quote_ident(Dialect::Sqlite, &table.database)
        );
        let sql: Option<String> = sqlx::query_scalar(&format!(
            "SELECT sql FROM {master} WHERE type = 'table' AND name = ?1"
        ))
        .bind(&table.table)
        .fetch_optional(&self.pool)
        .await?;
        Ok(sql
            .as_deref()
            .and_then(parse_create_table)
            .map(|parsed| parsed.unsupported)
            .unwrap_or_default())
    }

    /// 库里这张表以外的对象(视图、别的表、别的表上的触发器和索引),重建前查引用用。
    pub async fn schema_objects(&self, table: &TableRef) -> CoreResult<Vec<SchemaObject>> {
        let master = format!(
            "{}.sqlite_master",
            quote_ident(Dialect::Sqlite, &table.database)
        );
        let rows = sqlx::query(&format!(
            r"SELECT type, name, sql FROM {master}
              WHERE sql IS NOT NULL AND tbl_name <> ?1 COLLATE NOCASE
                AND name NOT LIKE 'sqlite\_%' ESCAPE '\'
              ORDER BY type, name"
        ))
        .bind(&table.table)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(SchemaObject {
                    kind: row.try_get("type")?,
                    name: row.try_get("name")?,
                    sql: row.try_get("sql")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(CoreError::from)
    }

    /// 有外键引用这张表的其它表。
    pub async fn referencing_tables(&self, table: &TableRef) -> CoreResult<Vec<String>> {
        let master = format!(
            "{}.sqlite_master",
            quote_ident(Dialect::Sqlite, &table.database)
        );
        let names = sqlx::query_scalar(&format!(
            "SELECT DISTINCT m.name FROM {master} m, pragma_foreign_key_list(m.name, ?2) f
             WHERE m.type = 'table' AND f.\"table\" = ?1 COLLATE NOCASE ORDER BY m.name"
        ))
        .bind(&table.table)
        .bind(&table.database)
        .fetch_all(&self.pool)
        .await?;
        Ok(names)
    }
}

fn column(row: &SqliteRow, parsed: &ParsedTable) -> Result<ColumnDef, sqlx::Error> {
    let name: String = row.try_get("name")?;
    let details = parsed
        .columns
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(&name));
    let hidden: i64 = row.try_get("hidden")?;
    let generated = details.and_then(|d| d.generated.clone()).or_else(|| {
        // 解析不出表达式时也要标成生成列,免得重建时当成普通列去插数据。
        (hidden > HIDDEN_VIRTUAL_TABLE).then(|| Generated {
            expression: String::new(),
            stored: hidden == HIDDEN_STORED,
        })
    });
    Ok(ColumnDef {
        id: existing_id(COLUMN_ID_PREFIX, &name),
        data_type: row.try_get("type")?,
        nullable: row.try_get::<i64, _>("notnull")? == 0,
        // PRAGMA 的 dflt_value 去掉了表达式外层的括号,优先用原文。
        default: match details.and_then(|d| d.default.clone()) {
            Some(text) => Some(text),
            None => row.try_get("dflt_value")?,
        },
        auto_increment: details.is_some_and(|d| d.autoincrement),
        comment: None,
        collation: details.and_then(|d| d.collation.clone()),
        on_update: None,
        generated,
        name,
    })
}

fn foreign_keys(
    rows: &[SqliteRow],
    parsed: &ParsedTable,
) -> Result<Vec<ForeignKeyDef>, sqlx::Error> {
    let mut keys: Vec<(i64, ForeignKeyDef)> = Vec::new();
    for row in rows {
        let id: i64 = row.try_get("id")?;
        if keys.last().is_none_or(|(last, _)| *last != id) {
            keys.push((
                id,
                ForeignKeyDef {
                    id: existing_id(FOREIGN_KEY_ID_PREFIX, &id.to_string()),
                    name: String::new(),
                    columns: Vec::new(),
                    ref_schema: None,
                    ref_table: row.try_get("table")?,
                    ref_columns: Vec::new(),
                    on_update: FkAction::parse(&row.try_get::<String, _>("on_update")?),
                    on_delete: FkAction::parse(&row.try_get::<String, _>("on_delete")?),
                },
            ));
        }
        let (_, key) = keys.last_mut().expect("刚刚放进去");
        let from: String = row.try_get("from")?;
        key.columns.push(existing_id(COLUMN_ID_PREFIX, &from));
        // `to` 为空表示引用对方的主键。
        if let Some(to) = row.try_get::<Option<String>, _>("to")? {
            key.ref_columns.push(to);
        }
    }
    // PRAGMA 不给外键名,按字段列表到 CREATE TABLE 原文里找;同时按声明顺序排(PRAGMA 是倒序)。
    let mut keys: Vec<ForeignKeyDef> = keys.into_iter().rev().map(|(_, key)| key).collect();
    for key in &mut keys {
        let columns: Vec<String> = key
            .columns
            .iter()
            .map(|id| id.trim_start_matches(COLUMN_ID_PREFIX).to_lowercase())
            .collect();
        if let Some((_, name)) = parsed.foreign_key_names.iter().find(|(c, _)| *c == columns) {
            key.name = name.clone();
        }
    }
    Ok(keys)
}
