//! PostgreSQL 的表结构读取与 DDL 执行。DDL 放在一个事务里,失败整体回滚。

use sqlx::Row;
use sqlx::postgres::{PgConnection, PgRow};

use super::postgres::PgSession;
use super::{finish_transaction, statement_failed, table_not_found};
use crate::design::{
    CHECK_ID_PREFIX, COLUMN_ID_PREFIX, CheckDef, ColumnDef, DdlStatement, FOREIGN_KEY_ID_PREFIX,
    FkAction, ForeignKeyDef, Generated, INDEX_ID_PREFIX, IndexColumn, IndexDef, TableDesign,
    TableOptions, existing_id,
};
use crate::error::{CoreError, CoreResult};
use crate::table::TableRef;

const TABLE_SQL: &str = "
    SELECT c.oid::bigint AS oid, obj_description(c.oid, 'pg_class') AS comment
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p')";

const COLUMNS_SQL: &str = "
    SELECT a.attname::text AS name,
           format_type(a.atttypid, a.atttypmod) AS data_type,
           NOT a.attnotnull AS nullable,
           pg_get_expr(d.adbin, d.adrelid) AS expression,
           a.attgenerated::text AS generated,
           a.attidentity <> '' AS is_identity,
           col_description(a.attrelid, a.attnum) AS comment,
           (SELECT quote_ident(cn.nspname) || '.' || quote_ident(co.collname)
            FROM pg_collation co JOIN pg_namespace cn ON cn.oid = co.collnamespace
            WHERE co.oid = a.attcollation AND a.attcollation <> t.typcollation) AS collation
    FROM pg_attribute a
    JOIN pg_type t ON t.oid = a.atttypid
    LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
    WHERE a.attrelid = $1::bigint::oid AND a.attnum > 0 AND NOT a.attisdropped
    ORDER BY a.attnum";

const CONSTRAINT_COLUMNS: &str = "
    SELECT con.conname::text AS name, con.contype::text AS kind, a.attname::text AS column_name,
           fn.nspname::text AS ref_schema, fc.relname::text AS ref_table,
           fa.attname::text AS ref_column,
           con.confupdtype::text AS on_update, con.confdeltype::text AS on_delete
    FROM pg_constraint con
    CROSS JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(attnum, fattnum, ord)
    JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum
    LEFT JOIN pg_class fc ON fc.oid = con.confrelid
    LEFT JOIN pg_namespace fn ON fn.oid = fc.relnamespace
    LEFT JOIN pg_attribute fa ON fa.attrelid = con.confrelid AND fa.attnum = k.fattnum
    WHERE con.conrelid = $1::bigint::oid AND con.contype IN ('p', 'f')
    ORDER BY con.contype, con.conname, k.ord";

// 表达式索引、部分索引、带 INCLUDE 的索引模型表达不了,整条定义原样给出。
const INDEXES_SQL: &str = "
    SELECT i.relname::text AS name, ix.indisunique AS is_unique, am.amname::text AS method,
           (ix.indexprs IS NOT NULL OR ix.indpred IS NOT NULL OR ix.indnatts <> ix.indnkeyatts) AS is_raw,
           pg_get_indexdef(ix.indexrelid) AS definition,
           EXISTS (SELECT 1 FROM pg_constraint con
                   WHERE con.conindid = ix.indexrelid AND con.conrelid = ix.indrelid
                     AND con.contype IN ('u', 'x')) AS is_constraint,
           a.attname::text AS column_name,
           (ix.indoption[k.ord - 1] & 1) = 1 AS descending
    FROM pg_index ix
    JOIN pg_class i ON i.oid = ix.indexrelid
    JOIN pg_am am ON am.oid = i.relam
    CROSS JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord)
    LEFT JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = k.attnum AND k.attnum > 0
    WHERE ix.indrelid = $1::bigint::oid AND NOT ix.indisprimary AND k.ord <= ix.indnkeyatts
    ORDER BY i.relname, k.ord";

const CHECKS_SQL: &str = "
    SELECT conname::text AS name, pg_get_constraintdef(oid) AS definition
    FROM pg_constraint WHERE conrelid = $1::bigint::oid AND contype = 'c'
    ORDER BY conname";

impl PgSession {
    pub async fn read_design(&self, table: &TableRef) -> CoreResult<TableDesign> {
        let pool = self.pool(&table.database).await?;
        let mut connection = pool.acquire().await?;
        read_design_on(&mut connection, table).await
    }

    pub async fn execute_ddl(
        &self,
        database: &str,
        statements: &[DdlStatement],
    ) -> CoreResult<usize> {
        let pool = self.pool(database).await?;
        let mut transaction = pool.begin().await?;
        let result = async {
            for (index, statement) in statements.iter().enumerate() {
                sqlx::query(&statement.sql)
                    .persistent(false)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|error| statement_failed(index, error))?;
            }
            Ok(statements.len() as u64)
        }
        .await;
        Ok(finish_transaction(transaction, result).await? as usize)
    }
}

fn column(row: &PgRow) -> Result<ColumnDef, sqlx::Error> {
    let name: String = row.try_get("name")?;
    let expression: Option<String> = row.try_get("expression")?;
    // attgenerated:'s' 存储列,'v' 虚拟列(PostgreSQL 18 起);空串是普通列。
    let generated_kind: String = row.try_get("generated")?;
    let (default, generated) = if generated_kind.is_empty() {
        (expression, None)
    } else {
        let generated = Generated {
            expression: expression.unwrap_or_default(),
            stored: generated_kind == "s",
        };
        (None, Some(generated))
    };
    Ok(ColumnDef {
        id: existing_id(COLUMN_ID_PREFIX, &name),
        name,
        data_type: row.try_get("data_type")?,
        nullable: row.try_get("nullable")?,
        default,
        auto_increment: row.try_get("is_identity")?,
        comment: super::non_empty(row.try_get("comment")?),
        // 只在和类型默认的不同时才有;改类型时要带上,否则会被重置成默认的。
        collation: row.try_get("collation")?,
        on_update: None,
        generated,
    })
}

fn indexes(rows: &[PgRow]) -> Result<Vec<IndexDef>, sqlx::Error> {
    let mut indexes: Vec<IndexDef> = Vec::new();
    for row in rows {
        let name: String = row.try_get("name")?;
        if indexes.last().is_none_or(|i| i.name != name) {
            let method: String = row.try_get("method")?;
            let raw: bool = row.try_get("is_raw")?;
            indexes.push(IndexDef {
                id: existing_id(INDEX_ID_PREFIX, &name),
                name,
                columns: Vec::new(),
                unique: row.try_get("is_unique")?,
                method: (method != "btree").then_some(method),
                definition: if raw {
                    row.try_get("definition")?
                } else {
                    None
                },
                is_constraint: row.try_get("is_constraint")?,
            });
        }
        let index = indexes.last_mut().expect("刚刚放进去");
        if index.definition.is_some() {
            continue;
        }
        if let Some(column) = row.try_get::<Option<String>, _>("column_name")? {
            index.columns.push(IndexColumn {
                column: existing_id(COLUMN_ID_PREFIX, &column),
                descending: row.try_get("descending")?,
                length: None,
            });
        }
    }
    Ok(indexes)
}

/// 主键与外键:一行一个字段,按约束名分组。
fn constraints(design: &mut TableDesign, schema: &str, rows: &[PgRow]) -> Result<(), sqlx::Error> {
    for row in rows {
        let name: String = row.try_get("name")?;
        let column = existing_id(COLUMN_ID_PREFIX, &row.try_get::<String, _>("column_name")?);
        if row.try_get::<String, _>("kind")? == "p" {
            design.primary_key_name = Some(name);
            design.primary_key.push(column);
            continue;
        }
        if design.foreign_keys.last().is_none_or(|k| k.name != name) {
            let ref_schema: String = row.try_get("ref_schema")?;
            design.foreign_keys.push(ForeignKeyDef {
                id: existing_id(FOREIGN_KEY_ID_PREFIX, &name),
                name,
                columns: Vec::new(),
                ref_schema: (ref_schema != schema).then_some(ref_schema),
                ref_table: row.try_get("ref_table")?,
                ref_columns: Vec::new(),
                on_update: action(&row.try_get::<String, _>("on_update")?),
                on_delete: action(&row.try_get::<String, _>("on_delete")?),
            });
        }
        let key = design.foreign_keys.last_mut().expect("刚刚放进去");
        key.columns.push(column);
        key.ref_columns.push(row.try_get("ref_column")?);
    }
    Ok(())
}

/// pg_constraint 里的动作代码。
fn action(code: &str) -> FkAction {
    match code {
        "r" => FkAction::Restrict,
        "c" => FkAction::Cascade,
        "n" => FkAction::SetNull,
        "d" => FkAction::SetDefault,
        _ => FkAction::NoAction,
    }
}

fn checks(rows: &[PgRow]) -> Result<Vec<CheckDef>, sqlx::Error> {
    rows.iter()
        .map(|row| {
            let name: String = row.try_get("name")?;
            let definition: String = row.try_get("definition")?;
            Ok(CheckDef {
                id: existing_id(CHECK_ID_PREFIX, &name),
                name: Some(name),
                expression: definition
                    .strip_prefix("CHECK ")
                    .unwrap_or(&definition)
                    .to_owned(),
            })
        })
        .collect()
}

async fn fetch_by_oid(
    connection: &mut PgConnection,
    sql: &str,
    oid: i64,
) -> Result<Vec<PgRow>, sqlx::Error> {
    sqlx::query(sql).bind(oid).fetch_all(&mut *connection).await
}

/// 在给定的连接上读表结构。转储要在 search_path 为空的事务里读(名字全带 schema),所以不直接用连接池。
pub(crate) async fn read_design_on(
    connection: &mut PgConnection,
    table: &TableRef,
) -> CoreResult<TableDesign> {
    let schema = table
        .schema
        .as_deref()
        .ok_or_else(|| CoreError::Unsupported("PostgreSQL 需要指定 schema".into()))?;
    let Some(table_row) = sqlx::query(TABLE_SQL)
        .bind(schema)
        .bind(&table.table)
        .fetch_optional(&mut *connection)
        .await?
    else {
        return Err(table_not_found(&format!("{schema}.{}", table.table)));
    };
    let oid: i64 = table_row.try_get("oid")?;
    let columns = fetch_by_oid(connection, COLUMNS_SQL, oid)
        .await?
        .iter()
        .map(column)
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let indexes = indexes(&fetch_by_oid(connection, INDEXES_SQL, oid).await?)?;
    let checks = checks(&fetch_by_oid(connection, CHECKS_SQL, oid).await?)?;
    let mut design = TableDesign {
        version: String::new(),
        table: table.clone(),
        columns,
        primary_key: Vec::new(),
        primary_key_name: None,
        indexes,
        foreign_keys: Vec::new(),
        checks,
        triggers: Vec::new(),
        options: TableOptions {
            comment: super::non_empty(table_row.try_get("comment")?),
            ..TableOptions::default()
        },
    };
    constraints(
        &mut design,
        schema,
        &fetch_by_oid(connection, CONSTRAINT_COLUMNS, oid).await?,
    )?;
    Ok(design)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_codes_map_to_rules() {
        assert_eq!(action("a"), FkAction::NoAction);
        assert_eq!(action("r"), FkAction::Restrict);
        assert_eq!(action("c"), FkAction::Cascade);
        assert_eq!(action("n"), FkAction::SetNull);
        assert_eq!(action("d"), FkAction::SetDefault);
    }
}
