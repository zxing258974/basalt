//! 表数据分页读取的 SELECT。

use super::filter::check_filter;
use super::{Dialect, qualified, quote_ident, table_name};
use crate::error::{CoreError, CoreResult};
use crate::table::{ColumnInfo, MAX_PAGE_SIZE, PageRequest, SortSpec, TableRef};

/// 生成一页数据的 SELECT。多取一行(`LIMIT limit + 1`)用来判断后面还有没有。
///
/// 列表按表结构显式列出,保证结果列与 `columns` 一一对应。排序时把主键追加在后面
/// 打破平局,否则 LIMIT / OFFSET 翻页可能重复或漏行;没有指定排序时按主键排。
pub fn select_page(
    dialect: Dialect,
    table: &TableRef,
    columns: &[ColumnInfo],
    request: &PageRequest,
) -> CoreResult<String> {
    if request.limit == 0 || request.limit > MAX_PAGE_SIZE {
        return Err(CoreError::InvalidFilter(format!(
            "每页行数要在 1 到 {MAX_PAGE_SIZE} 之间。"
        )));
    }
    let mut sql = select_unpaged(dialect, table, columns, request)?;
    sql.push_str(&format!(
        " LIMIT {} OFFSET {}",
        u64::from(request.limit) + 1,
        request.offset
    ));
    Ok(sql)
}

/// 导出整张表或筛选后的全部结果:和读一页一样显式列出列、带筛选与排序(主键打破平局),但不分页。
pub fn select_all(
    dialect: Dialect,
    table: &TableRef,
    columns: &[ColumnInfo],
    filter: Option<&str>,
    sort: Option<&SortSpec>,
) -> CoreResult<String> {
    let request = PageRequest {
        offset: 0,
        limit: 1,
        sort: sort.cloned(),
        filter: filter.map(str::to_owned),
    };
    select_unpaged(dialect, table, columns, &request)
}

fn select_unpaged(
    dialect: Dialect,
    table: &TableRef,
    columns: &[ColumnInfo],
    request: &PageRequest,
) -> CoreResult<String> {
    if columns.is_empty() {
        return Err(CoreError::Unsupported(
            "这张表没有任何列，无法读取数据。".into(),
        ));
    }
    let mut sql = format!(
        "SELECT {} FROM {}",
        select_list(dialect, columns),
        table_name(dialect, table)
    );
    if let Some(filter) = request
        .filter
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    {
        check_filter(dialect, filter).map_err(CoreError::InvalidFilter)?;
        // 条件单独成行:末尾的 `--` 注释不会吞掉右括号和 LIMIT。
        sql.push_str(&format!(" WHERE (\n{filter}\n)"));
    }
    let order = order_by(dialect, table, columns, request)?;
    if !order.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&order.join(", "));
    }
    Ok(sql)
}

fn select_list(dialect: Dialect, columns: &[ColumnInfo]) -> String {
    columns
        .iter()
        .map(|column| {
            let name = quote_ident(dialect, &column.name);
            if column.select_as_text {
                format!("{name}::text AS {name}")
            } else {
                name
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn order_by(
    dialect: Dialect,
    table: &TableRef,
    columns: &[ColumnInfo],
    request: &PageRequest,
) -> CoreResult<Vec<String>> {
    // PostgreSQL 里不加限定的列名会先匹配输出列,而输出列可能已经转成了 text。
    let reference = |name: &str| match dialect {
        Dialect::Postgres => qualified(dialect, &[&table.table, name]),
        Dialect::Mysql | Dialect::Sqlite => quote_ident(dialect, name),
    };
    let mut terms = Vec::new();
    let mut sorted_by = None;
    if let Some(sort) = &request.sort {
        let column = columns
            .iter()
            .find(|c| c.name == sort.column)
            .ok_or_else(|| {
                CoreError::InvalidFilter(format!("表里没有列「{}」，无法排序。", sort.column))
            })?;
        let direction = if sort.descending { " DESC" } else { "" };
        terms.push(format!("{}{direction}", reference(&column.name)));
        sorted_by = Some(column.name.as_str());
    }
    terms.extend(
        columns
            .iter()
            .filter(|c| c.is_primary_key && Some(c.name.as_str()) != sorted_by)
            .map(|c| reference(&c.name)),
    );
    Ok(terms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::ValueType;

    fn pk(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo {
            is_primary_key: true,
            nullable: false,
            value_type: ValueType::Integer { unsigned: false },
            ..ColumnInfo::new(name, data_type)
        }
    }

    fn mysql_table() -> TableRef {
        TableRef {
            database: "shop".into(),
            schema: None,
            table: "orders".into(),
        }
    }

    fn request(offset: u64, limit: u32) -> PageRequest {
        PageRequest {
            offset,
            limit,
            sort: None,
            filter: None,
        }
    }

    #[test]
    fn mysql_page_lists_columns_and_orders_by_primary_key() {
        let columns = [pk("id", "int"), ColumnInfo::new("name", "varchar(50)")];
        let sql =
            select_page(Dialect::Mysql, &mysql_table(), &columns, &request(200, 100)).unwrap();
        assert_eq!(
            sql,
            "SELECT `id`, `name` FROM `shop`.`orders` ORDER BY `id` LIMIT 101 OFFSET 200"
        );
    }

    #[test]
    fn sort_column_comes_first_and_primary_key_breaks_ties() {
        let columns = [
            pk("order_id", "int"),
            pk("line", "int"),
            ColumnInfo::new("name", "text"),
        ];
        let sorted = PageRequest {
            sort: Some(SortSpec {
                column: "name".into(),
                descending: true,
            }),
            ..request(0, 50)
        };
        let sql = select_page(Dialect::Mysql, &mysql_table(), &columns, &sorted).unwrap();
        assert!(
            sql.ends_with("ORDER BY `name` DESC, `order_id`, `line` LIMIT 51 OFFSET 0"),
            "{sql}"
        );

        // 按主键列本身排序时不重复追加它。
        let by_line = PageRequest {
            sort: Some(SortSpec {
                column: "line".into(),
                descending: false,
            }),
            ..request(0, 50)
        };
        let sql = select_page(Dialect::Mysql, &mysql_table(), &columns, &by_line).unwrap();
        assert!(sql.contains("ORDER BY `line`, `order_id` LIMIT"), "{sql}");
    }

    #[test]
    fn filter_is_wrapped_in_parentheses_on_its_own_lines() {
        let columns = [pk("id", "int")];
        let filtered = PageRequest {
            filter: Some("  name = 'x' -- 只看 x  ".into()),
            ..request(0, 10)
        };
        let sql = select_page(
            Dialect::Sqlite,
            &TableRef {
                database: "main".into(),
                schema: None,
                table: "notes".into(),
            },
            &columns,
            &filtered,
        )
        .unwrap();
        assert_eq!(
            sql,
            "SELECT \"id\" FROM \"main\".\"notes\" WHERE (\nname = 'x' -- 只看 x\n) ORDER BY \"id\" LIMIT 11 OFFSET 0"
        );

        let blank = PageRequest {
            filter: Some("   ".into()),
            ..request(0, 10)
        };
        let sql = select_page(Dialect::Mysql, &mysql_table(), &columns, &blank).unwrap();
        assert!(!sql.contains("WHERE"), "{sql}");
    }

    #[test]
    fn postgres_reads_undecodable_columns_as_text_and_qualifies_order_by() {
        let columns = [
            pk("id", "integer"),
            ColumnInfo {
                select_as_text: true,
                ..ColumnInfo::new("tags", "text[]")
            },
        ];
        let table = TableRef {
            database: "basalt_it".into(),
            schema: Some("sales".into()),
            table: "orders".into(),
        };
        let sorted = PageRequest {
            sort: Some(SortSpec {
                column: "tags".into(),
                descending: true,
            }),
            ..request(0, 50)
        };
        let sql = select_page(Dialect::Postgres, &table, &columns, &sorted).unwrap();
        // 不加限定的话,ORDER BY "tags" 会按转成 text 的输出列排序。
        assert_eq!(
            sql,
            "SELECT \"id\", \"tags\"::text AS \"tags\" FROM \"sales\".\"orders\" \
             ORDER BY \"orders\".\"tags\" DESC, \"orders\".\"id\" LIMIT 51 OFFSET 0"
        );
    }

    #[test]
    fn select_all_keeps_filter_and_order_but_no_limit() {
        let columns = [pk("id", "int"), ColumnInfo::new("name", "text")];
        let sort = SortSpec {
            column: "name".into(),
            descending: true,
        };
        let sql = select_all(
            Dialect::Mysql,
            &mysql_table(),
            &columns,
            Some("id > 5"),
            Some(&sort),
        )
        .unwrap();
        assert_eq!(
            sql,
            "SELECT `id`, `name` FROM `shop`.`orders` WHERE (\nid > 5\n) ORDER BY `name` DESC, `id`"
        );
        let error = select_all(
            Dialect::Mysql,
            &mysql_table(),
            &columns,
            Some("1; DROP TABLE t"),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), "invalid_filter");
    }

    #[test]
    fn tables_without_primary_key_have_no_default_order() {
        let columns = [ColumnInfo::new("a", "text"), ColumnInfo::new("b", "text")];
        let sql = select_page(Dialect::Mysql, &mysql_table(), &columns, &request(0, 10)).unwrap();
        assert_eq!(
            sql,
            "SELECT `a`, `b` FROM `shop`.`orders` LIMIT 11 OFFSET 0"
        );
    }

    #[test]
    fn rejects_unknown_sort_columns_bad_limits_and_bad_filters() {
        let columns = [pk("id", "int")];
        let unknown = PageRequest {
            sort: Some(SortSpec {
                column: "nope".into(),
                descending: false,
            }),
            ..request(0, 10)
        };
        let error = select_page(Dialect::Mysql, &mysql_table(), &columns, &unknown).unwrap_err();
        assert_eq!(error.code(), "invalid_filter");
        assert!(error.to_string().contains("nope"), "{error}");

        for limit in [0, crate::table::MAX_PAGE_SIZE + 1] {
            let error = select_page(Dialect::Mysql, &mysql_table(), &columns, &request(0, limit))
                .unwrap_err();
            assert_eq!(error.code(), "invalid_filter", "{limit}");
        }

        let injected = PageRequest {
            filter: Some("1 = 1; DROP TABLE orders".into()),
            ..request(0, 10)
        };
        let error = select_page(Dialect::Mysql, &mysql_table(), &columns, &injected).unwrap_err();
        assert_eq!(error.code(), "invalid_filter");

        // PostgreSQL 允许零列的表,没法生成 SELECT 列表。
        let error = select_page(Dialect::Mysql, &mysql_table(), &[], &request(0, 10)).unwrap_err();
        assert_eq!(error.code(), "unsupported");
    }
}
