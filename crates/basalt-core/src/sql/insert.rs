//! 导入用的语句:多行 INSERT、追加或更新(upsert)、按匹配键 UPDATE / DELETE 与清空表。
//! 值已经按目标列转换好(见 [`crate::transfer::convert`]),这里只拼语句:一律参数化,
//! PostgreSQL 的参数带 `::列类型`,和表格编辑一样。

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

/// PostgreSQL 的参数按列类型转换;MySQL、SQLite 不用。
fn cast(dialect: Dialect, column: &ColumnInfo) -> Option<String> {
    match dialect {
        Dialect::Postgres => column.write_cast.clone(),
        Dialect::Mysql | Dialect::Sqlite => None,
    }
}

fn values_clause(
    dialect: Dialect,
    table: &TableRef,
    columns: &[&ColumnInfo],
    rows: Vec<Vec<BindValue>>,
) -> Builder {
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
            builder.param((value, cast(dialect, column)));
        }
        builder.sql(")");
    }
    builder
}

/// `INSERT INTO 表 (列…) VALUES (…), (…)`。每行的值个数必须和列数相同。
pub fn insert_rows(
    dialect: Dialect,
    table: &TableRef,
    columns: &[&ColumnInfo],
    rows: Vec<Vec<BindValue>>,
) -> Statement {
    values_clause(dialect, table, columns, rows).finish()
}

/// 追加或更新:按 `keys`(主键或唯一键在 `columns` 里的位置)已有的行改掉其余的列,没有的插入。
/// 除键以外没有别的列时,已有的行保持不变。
///
/// MySQL 的 `ON DUPLICATE KEY UPDATE` 不指定键:任何一个唯一键冲突都算「已有」。
pub fn upsert_rows(
    dialect: Dialect,
    table: &TableRef,
    columns: &[&ColumnInfo],
    keys: &[usize],
    rows: Vec<Vec<BindValue>>,
) -> Statement {
    let quoted = |index: usize| quote_ident(dialect, &columns[index].name);
    let rest: Vec<usize> = (0..columns.len()).filter(|i| !keys.contains(i)).collect();
    let mut builder = values_clause(dialect, table, columns, rows);
    let tail = match dialect {
        Dialect::Mysql => {
            // 没有别的列可改时让键等于自己:语句合法,已有的行不变。
            let assignments: Vec<String> = if rest.is_empty() {
                keys.iter()
                    .map(|&i| format!("{0} = {0}", quoted(i)))
                    .collect()
            } else {
                rest.iter()
                    .map(|&i| format!("{0} = VALUES({0})", quoted(i)))
                    .collect()
            };
            format!(" ON DUPLICATE KEY UPDATE {}", assignments.join(", "))
        }
        Dialect::Postgres | Dialect::Sqlite => {
            let target: Vec<String> = keys.iter().map(|&i| quoted(i)).collect();
            let action = if rest.is_empty() {
                "DO NOTHING".to_owned()
            } else {
                let assignments: Vec<String> = rest
                    .iter()
                    .map(|&i| format!("{0} = excluded.{0}", quoted(i)))
                    .collect();
                format!("DO UPDATE SET {}", assignments.join(", "))
            };
            format!(" ON CONFLICT ({}) {action}", target.join(", "))
        }
    };
    builder.sql(&tail);
    builder.finish()
}

/// `… WHERE 键1 = ? AND 键2 = ?`。
fn key_condition(builder: &mut Builder, dialect: Dialect, keys: &[(&ColumnInfo, BindValue)]) {
    builder.sql(" WHERE ");
    for (index, (column, value)) in keys.iter().enumerate() {
        if index > 0 {
            builder.sql(" AND ");
        }
        builder.sql(&format!("{} = ", quote_ident(dialect, &column.name)));
        builder.param((value.clone(), cast(dialect, column)));
    }
}

/// 更新:按匹配键改一行(键不唯一时可能改到多行)。`set` 不能为空。
pub fn update_row(
    dialect: Dialect,
    table: &TableRef,
    set: &[(&ColumnInfo, BindValue)],
    keys: &[(&ColumnInfo, BindValue)],
) -> Statement {
    debug_assert!(!set.is_empty() && !keys.is_empty());
    let mut builder = Builder::new(dialect);
    builder.sql(&format!("UPDATE {} SET ", table_name(dialect, table)));
    for (index, (column, value)) in set.iter().enumerate() {
        if index > 0 {
            builder.sql(", ");
        }
        builder.sql(&format!("{} = ", quote_ident(dialect, &column.name)));
        builder.param((value.clone(), cast(dialect, column)));
    }
    key_condition(&mut builder, dialect, keys);
    builder.finish()
}

/// 删除:按匹配键删掉对应的行。
pub fn delete_row(
    dialect: Dialect,
    table: &TableRef,
    keys: &[(&ColumnInfo, BindValue)],
) -> Statement {
    debug_assert!(!keys.is_empty());
    let mut builder = Builder::new(dialect);
    builder.sql(&format!("DELETE FROM {}", table_name(dialect, table)));
    key_condition(&mut builder, dialect, keys);
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
    fn upsert_follows_each_dialect() {
        let id = ColumnInfo::new("id", "int");
        let name = ColumnInfo::new("name", "text");
        let rows = || vec![vec![BindValue::Int(1), BindValue::Text("a".into())]];
        let mysql = upsert_rows(
            Dialect::Mysql,
            &table(Dialect::Mysql),
            &[&id, &name],
            &[0],
            rows(),
        );
        assert_eq!(
            mysql.sql(),
            "INSERT INTO `shop`.`users` (`id`, `name`) VALUES (?, ?) ON DUPLICATE KEY UPDATE `name` = VALUES(`name`)"
        );
        let pg = upsert_rows(
            Dialect::Postgres,
            &table(Dialect::Postgres),
            &[&id, &name],
            &[0],
            rows(),
        );
        assert_eq!(
            pg.sql(),
            "INSERT INTO \"public\".\"users\" (\"id\", \"name\") VALUES ($1, $2) ON CONFLICT (\"id\") DO UPDATE SET \"name\" = excluded.\"name\""
        );
        // 只有键:已有的行不动。
        let only_key = || vec![vec![BindValue::Int(1)]];
        let mysql = upsert_rows(
            Dialect::Mysql,
            &table(Dialect::Mysql),
            &[&id],
            &[0],
            only_key(),
        );
        assert!(mysql.sql().ends_with("ON DUPLICATE KEY UPDATE `id` = `id`"));
        let sqlite = upsert_rows(
            Dialect::Sqlite,
            &table(Dialect::Sqlite),
            &[&id],
            &[0],
            only_key(),
        );
        assert!(sqlite.sql().ends_with("ON CONFLICT (\"id\") DO NOTHING"));
    }

    #[test]
    fn update_and_delete_match_on_every_key_column() {
        let a = ColumnInfo {
            write_cast: Some("integer".into()),
            ..ColumnInfo::new("a", "integer")
        };
        let b = ColumnInfo::new("b", "text");
        let note = ColumnInfo::new("note", "text");
        let keys = [(&a, BindValue::Int(1)), (&b, BindValue::Text("x".into()))];
        let update = update_row(
            Dialect::Postgres,
            &table(Dialect::Postgres),
            &[(&note, BindValue::Null)],
            &keys,
        );
        assert_eq!(
            update.sql(),
            "UPDATE \"public\".\"users\" SET \"note\" = $1 WHERE \"a\" = $2::integer AND \"b\" = $3"
        );
        let delete = delete_row(Dialect::Mysql, &table(Dialect::Mysql), &keys);
        assert_eq!(
            delete.preview(),
            "DELETE FROM `shop`.`users` WHERE `a` = 1 AND `b` = 'x'"
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
