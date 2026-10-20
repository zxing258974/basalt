//! SQLite 的表数据。SQLite 是动态类型,同一列的值可以是任意存储类型,
//! 所以按每个值自己的存储类型解码;写入时传文本,由列的类型亲和性转换。

use sqlx::query::Query;
use sqlx::sqlite::{Sqlite, SqliteArguments, SqliteRow};
use sqlx::{Row, TypeInfo, ValueRef};

use super::sqlite::SqliteSession;
use super::{check_affected, finish_transaction, statement_failed, table_not_found};
use crate::error::CoreResult;
use crate::sql::bind::BindValue;
use crate::sql::dml::Statement;
use crate::table::{ColumnInfo, TableRef, ValueType};
use crate::value::CellValue;

/// table_xinfo 的 hidden:1 是虚拟表的隐藏列(SELECT * 不返回),2、3 是生成列。
const HIDDEN_VIRTUAL: i64 = 1;

impl SqliteSession {
    pub async fn columns(&self, table: &TableRef) -> CoreResult<Vec<ColumnInfo>> {
        let rows = sqlx::query(
            "SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?1, ?2)",
        )
        .bind(&table.table)
        .bind(&table.database)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Err(table_not_found(&table.table));
        }
        let primary_count = rows
            .iter()
            .filter(|row| row.try_get::<i64, _>("pk").unwrap_or(0) > 0)
            .count();
        let mut columns = Vec::with_capacity(rows.len());
        for row in &rows {
            let hidden: i64 = row.try_get("hidden")?;
            if hidden == HIDDEN_VIRTUAL {
                continue;
            }
            let declared: String = row.try_get("type")?;
            let is_primary_key = row.try_get::<i64, _>("pk")? > 0;
            // 单列 INTEGER PRIMARY KEY 是 rowid 的别名,不填会自动分配。
            let is_rowid =
                is_primary_key && primary_count == 1 && declared.eq_ignore_ascii_case("INTEGER");
            columns.push(ColumnInfo {
                is_primary_key,
                nullable: row.try_get::<i64, _>("notnull")? == 0,
                has_default: row.try_get::<Option<String>, _>("dflt_value")?.is_some() || is_rowid,
                is_generated: hidden > HIDDEN_VIRTUAL,
                value_type: affinity(&declared),
                ..ColumnInfo::new(&row.try_get::<String, _>("name")?, &declared)
            });
        }
        Ok(columns)
    }

    /// SQLite 会把一个字符串里的多条语句都执行掉,筛选条件的词法检查之外再加一道:
    /// 读页时连接临时设成只读,混进来的写语句会直接失败。
    pub async fn fetch_rows(
        &self,
        sql: &str,
        columns: &[ColumnInfo],
    ) -> CoreResult<Vec<Vec<CellValue>>> {
        let mut connection = self.pool.acquire().await?;
        sqlx::query("PRAGMA query_only = ON")
            .execute(&mut *connection)
            .await?;
        let rows = sqlx::query(sql)
            .persistent(false)
            .fetch_all(&mut *connection)
            .await;
        if let Err(error) = sqlx::query("PRAGMA query_only = OFF")
            .execute(&mut *connection)
            .await
        {
            // 恢复不了就别把这条只读连接还回池里。
            log::warn!("恢复 SQLite 连接的可写状态失败，丢弃这条连接：{error}");
            drop(connection.detach());
        }
        let rows = rows?;
        Ok(rows
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|index| decode_cell(row, index))
                    .collect()
            })
            .collect())
    }

    pub async fn execute(&self, statements: &[Statement]) -> CoreResult<u64> {
        let mut transaction = self.pool.begin().await?;
        let result = async {
            let mut affected = 0;
            for (index, statement) in statements.iter().enumerate() {
                let sql = statement.sql();
                let query = statement
                    .params()
                    .into_iter()
                    .fold(sqlx::query(&sql).persistent(false), bind);
                let done = query
                    .execute(&mut *transaction)
                    .await
                    .map_err(|error| statement_failed(index, error))?;
                check_affected(index, done.rows_affected())?;
                affected += done.rows_affected();
            }
            Ok(affected)
        }
        .await;
        finish_transaction(transaction, result).await
    }
}

fn bind<'q>(
    query: Query<'q, Sqlite, SqliteArguments<'q>>,
    value: BindValue,
) -> Query<'q, Sqlite, SqliteArguments<'q>> {
    match value {
        BindValue::Null => query.bind(None::<String>),
        BindValue::Bool(b) => query.bind(b),
        BindValue::Int(n) => query.bind(n),
        BindValue::UInt(n) => match i64::try_from(n) {
            Ok(n) => query.bind(n),
            Err(_) => query.bind(n.to_string()),
        },
        BindValue::Float(f) => query.bind(f),
        BindValue::Decimal(d) => query.bind(d.to_string()),
        BindValue::Text(text) => query.bind(text),
        BindValue::Bytes(bytes) => query.bind(bytes),
    }
}

/// SQLite 的类型亲和性规则(按声明类型里的关键字,顺序有意义)。
/// 写入只关心 BLOB:以 0x 开头的输入按十六进制存成字节。
fn affinity(declared: &str) -> ValueType {
    let upper = declared.to_ascii_uppercase();
    if upper.contains("INT") {
        ValueType::Integer { unsigned: false }
    } else if ["CHAR", "CLOB", "TEXT"].iter().any(|k| upper.contains(k)) {
        ValueType::Text
    } else if upper.contains("BLOB") || upper.trim().is_empty() {
        ValueType::Binary
    } else if ["REAL", "FLOA", "DOUB"].iter().any(|k| upper.contains(k)) {
        ValueType::Float
    } else {
        ValueType::Decimal
    }
}

/// 按值的存储类型解码。NULL 值的 type_info 是列的声明类型,所以先判空。
pub(crate) fn decode_cell(row: &SqliteRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::unsupported("?");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_owned();
    let decoded = match type_name.as_str() {
        "INTEGER" => row.try_get_unchecked::<i64, _>(index).map(CellValue::Int),
        "REAL" => row.try_get_unchecked::<f64, _>(index).map(CellValue::Float),
        "TEXT" => row
            .try_get_unchecked::<String, _>(index)
            .map(CellValue::Text),
        "BLOB" => row
            .try_get_unchecked::<Vec<u8>, _>(index)
            .map(CellValue::Bytes),
        _ => return CellValue::Unsupported { type_name },
    };
    decoded.unwrap_or_else(|error| {
        log::warn!("解码 SQLite {type_name} 失败：{error}");
        CellValue::Unsupported { type_name }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affinity_follows_sqlite_rules() {
        assert_eq!(affinity("INTEGER"), ValueType::Integer { unsigned: false });
        assert_eq!(affinity("varchar(20)"), ValueType::Text);
        assert_eq!(affinity("BLOB"), ValueType::Binary);
        assert_eq!(affinity(""), ValueType::Binary);
        assert_eq!(affinity("DOUBLE PRECISION"), ValueType::Float);
        assert_eq!(affinity("DECIMAL(10,2)"), ValueType::Decimal);
        // 「CHARINT」先命中 INT,和 SQLite 一致。
        assert_eq!(affinity("CHARINT"), ValueType::Integer { unsigned: false });
    }
}
