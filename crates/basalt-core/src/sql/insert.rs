//! 导入用的多行 INSERT 与清空表。值已经按目标列转换好(见 [`crate::transfer::convert`]),
//! 这里只拼语句:一律参数化,PostgreSQL 的参数带 `::列类型`,和表格编辑一样。

use super::bind::BindValue;
use super::dml::{Builder, Statement};
use super::{Dialect, quote_ident, table_name};
use crate::table::{ColumnInfo, TableRef};

/// 一条语句最多能带多少个参数。MySQL、PostgreSQL 的协议里参数个数是 16 位无符号数;
/// SQLite 3.32 起默认上限是 32766。
pub fn max_params(dialect: Dialect) -> usize {
    match dialect {
        Dialect::Mysql | Dialect::Postgres => 65_535,
        Dialect::Sqlite => 32_766,
    }
}

/// 一条 INSERT 实际能放几行:不超过用户要的批大小,也不超过参数上限。至少一行。
pub fn rows_per_statement(dialect: Dialect, column_count: usize, wanted: usize) -> usize {
    let by_params = max_params(dialect) / column_count.max(1);
    wanted.min(by_params).max(1)
}

/// `INSERT INTO 表 (列…) VALUES (…), (…)`。每行的值个数必须和列数相同。
pub fn insert_rows(
    dialect: Dialect,
    table: &TableRef,
    columns: &[&ColumnInfo],
    rows: Vec<Vec<BindValue>>,
) -> Statement {
    let names: Vec<String> = columns
        .iter()
        .map(|c| quote_ident(dialect, &c.name))
        .collect();
    let mut builder = Builder::new(dialect);
    builder.sql(&format!(
        "INSERT INTO {} ({}) VALUES ",
        table_name(dialect, table),
        names.join(", ")
    ));
    for (row_index, row) in rows.into_iter().enumerate() {
        debug_assert_eq!(row.len(), columns.len());
        builder.sql(if row_index == 0 { "(" } else { ", (" });
        for (index, (value, column)) in row.into_iter().zip(columns).enumerate() {
            if index > 0 {
                builder.sql(", ");
            }
            let cast = match dialect {
                Dialect::Postgres => column.write_cast.clone(),
                Dialect::Mysql | Dialect::Sqlite => None,
            };
            builder.param((value, cast));
        }
        builder.sql(")");
    }
    builder.finish()
}

/// 导入前清空表:用 DELETE 而不是 TRUNCATE——DELETE 能放进事务里回滚,
/// 也不需要额外权限、不会被外键挡住整表截断。
pub fn clear_table(dialect: Dialect, table: &TableRef) -> String {
    format!("DELETE FROM {}", table_name(dialect, table))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(dialect: Dialect) -> TableRef {
        TableRef {
            database: "shop".into(),
            schema: (dialect == Dialect::Postgres).then(|| "public".into()),
            table: "users".into(),
        }
    }

    #[test]
    fn mysql_batches_rows_with_question_marks() {
        let id = ColumnInfo::new("id", "int");
        let name = ColumnInfo::new("name", "varchar(20)");
        let statement = insert_rows(
            Dialect::Mysql,
            &table(Dialect::Mysql),
            &[&id, &name],
            vec![
                vec![BindValue::Int(1), BindValue::Text("张三".into())],
                vec![BindValue::Int(2), BindValue::Null],
            ],
        );
        assert_eq!(
            statement.sql(),
            "INSERT INTO `shop`.`users` (`id`, `name`) VALUES (?, ?), (?, ?)"
        );
        assert_eq!(statement.params().len(), 4);
        assert_eq!(
            statement.preview(),
            "INSERT INTO `shop`.`users` (`id`, `name`) VALUES (1, '张三'), (2, NULL)"
        );
    }

    #[test]
    fn postgres_casts_each_parameter_to_the_column_type() {
        let id = ColumnInfo {
            write_cast: Some("integer".into()),
            ..ColumnInfo::new("id", "integer")
        };
        let note = ColumnInfo::new("note", "text");
        let statement = insert_rows(
            Dialect::Postgres,
            &table(Dialect::Postgres),
            &[&id, &note],
            vec![
                vec![BindValue::Text("1".into()), BindValue::Text("a".into())],
                vec![BindValue::Text("2".into()), BindValue::Null],
            ],
        );
        assert_eq!(
            statement.sql(),
            "INSERT INTO \"public\".\"users\" (\"id\", \"note\") VALUES ($1::integer, $2), ($3::integer, $4)"
        );
    }

    #[test]
    fn rows_per_statement_respects_parameter_limits() {
        assert_eq!(rows_per_statement(Dialect::Mysql, 10, 500), 500);
        assert_eq!(rows_per_statement(Dialect::Sqlite, 100, 500), 327);
        assert_eq!(rows_per_statement(Dialect::Postgres, 70_000, 500), 1);
        assert_eq!(rows_per_statement(Dialect::Mysql, 0, 0), 1);
    }

    #[test]
    fn clear_uses_delete_so_it_can_roll_back() {
        assert_eq!(
            clear_table(Dialect::Sqlite, &table(Dialect::Sqlite)),
            "DELETE FROM \"shop\".\"users\""
        );
        assert_eq!(
            clear_table(Dialect::Postgres, &table(Dialect::Postgres)),
            "DELETE FROM \"public\".\"users\""
        );
    }
}
