//! 表格编辑生成的 UPDATE / INSERT / DELETE。纯函数,只依赖列信息。
//!
//! 一律参数化,按主键定位行。PostgreSQL 的参数全部以文本传入并显式转换成列类型
//! (`$1::integer`),由数据库自己解析,不在客户端猜格式;MySQL 的主键按原值的类型
//! 绑定,避免字符串与整数比较时退化成浮点比较而匹配错行。

use std::fmt::Write;

use super::bind::{BindValue, convert_input, key_param, literal};
use super::{Dialect, quote_ident, table_name};
use crate::error::{CoreError, CoreResult};
use crate::table::{ColumnInfo, EditValues, RowChange, RowKey, TableRef, read_only_reason};
use crate::value::CellValue;

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Sql(String),
    Param {
        value: BindValue,
        cast: Option<String>,
    },
}

/// 一条待执行的语句。同一份内容既能生成带占位符的 SQL,也能生成给人看的预览。
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    dialect: Dialect,
    parts: Vec<Part>,
}

impl Statement {
    /// 带占位符的 SQL。
    pub fn sql(&self) -> String {
        self.render(|out, index, value, cast| {
            let _ = value;
            match self.dialect {
                Dialect::Postgres => {
                    let _ = write!(out, "${index}");
                    if let Some(cast) = cast {
                        let _ = write!(out, "::{cast}");
                    }
                }
                Dialect::Mysql | Dialect::Sqlite => out.push('?'),
            }
        })
    }

    pub fn params(&self) -> Vec<BindValue> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Param { value, .. } => Some(value.clone()),
                Part::Sql(_) => None,
            })
            .collect()
    }

    /// 参数内联成字面量的 SQL,只用于显示。
    pub fn preview(&self) -> String {
        self.render(|out, _, value, cast| {
            out.push_str(&literal(self.dialect, value));
            if let Some(cast) = cast {
                let _ = write!(out, "::{cast}");
            }
        })
    }

    fn render(
        &self,
        mut param: impl FnMut(&mut String, usize, &BindValue, Option<&str>),
    ) -> String {
        let mut out = String::new();
        let mut index = 0;
        for part in &self.parts {
            match part {
                Part::Sql(sql) => out.push_str(sql),
                Part::Param { value, cast } => {
                    index += 1;
                    param(&mut out, index, value, cast.as_deref());
                }
            }
        }
        out
    }
}

struct Builder {
    dialect: Dialect,
    parts: Vec<Part>,
}

impl Builder {
    fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            parts: Vec::new(),
        }
    }

    fn sql(&mut self, sql: &str) {
        match self.parts.last_mut() {
            Some(Part::Sql(last)) => last.push_str(sql),
            _ => self.parts.push(Part::Sql(sql.to_owned())),
        }
    }

    fn param(&mut self, (value, cast): (BindValue, Option<String>)) {
        self.parts.push(Part::Param { value, cast });
    }

    fn finish(self) -> Statement {
        Statement {
            dialect: self.dialect,
            parts: self.parts,
        }
    }
}

/// 生成一条修改时需要的上下文。`number` 是这条修改在用户提交里的序号,用于报错。
struct Change<'a> {
    dialect: Dialect,
    target: &'a str,
    columns: &'a [ColumnInfo],
    number: usize,
}

impl Change<'_> {
    fn error(&self, message: impl std::fmt::Display) -> CoreError {
        CoreError::InvalidEdit(format!("第 {} 处修改：{message}", self.number))
    }

    fn quote(&self, column: &ColumnInfo) -> String {
        quote_ident(self.dialect, &column.name)
    }

    /// 按表的列顺序取出要写的列,拒绝不存在的列和生成列。
    fn writable<'v>(
        &self,
        values: &'v EditValues,
    ) -> CoreResult<Vec<(&ColumnInfo, &'v Option<String>)>> {
        if let Some(unknown) = values
            .keys()
            .find(|name| !self.columns.iter().any(|c| &c.name == *name))
        {
            return Err(self.error(format_args!("表里没有列「{unknown}」。")));
        }
        self.columns
            .iter()
            .filter_map(|column| values.get(&column.name).map(|value| (column, value)))
            .map(|(column, value)| {
                if column.is_generated {
                    Err(self.error(format_args!("「{}」是生成列，不能写入。", column.name)))
                } else {
                    Ok((column, value))
                }
            })
            .collect()
    }

    fn value(
        &self,
        column: &ColumnInfo,
        input: &Option<String>,
    ) -> CoreResult<(BindValue, Option<String>)> {
        let value = match input {
            None => BindValue::Null,
            Some(text) => {
                convert_input(self.dialect, column.value_type, text).map_err(|reason| {
                    self.error(format_args!("「{}」{reason}：{text}", column.name))
                })?
            }
        };
        Ok((value, self.cast(column)))
    }

    fn cast(&self, column: &ColumnInfo) -> Option<String> {
        match self.dialect {
            Dialect::Postgres => column.write_cast.clone(),
            Dialect::Mysql | Dialect::Sqlite => None,
        }
    }

    /// `WHERE 主键 = ?`。主键必须完整,不多不少。
    fn locate(&self, builder: &mut Builder, key: &RowKey) -> CoreResult<()> {
        let primary: Vec<&ColumnInfo> = self.columns.iter().filter(|c| c.is_primary_key).collect();
        let complete =
            key.len() == primary.len() && primary.iter().all(|c| key.contains_key(&c.name));
        if !complete {
            let names = |list: Vec<&str>| list.join("、");
            return Err(self.error(format_args!(
                "定位行需要完整的主键（{}），收到的是（{}）。",
                names(primary.iter().map(|c| c.name.as_str()).collect()),
                names(key.keys().map(String::as_str).collect()),
            )));
        }
        builder.sql(" WHERE ");
        for (index, column) in primary.into_iter().enumerate() {
            if index > 0 {
                builder.sql(" AND ");
            }
            builder.sql(&self.quote(column));
            match &key[&column.name] {
                CellValue::Null => builder.sql(" IS NULL"),
                value => {
                    let bound = key_param(self.dialect, value).ok_or_else(|| {
                        self.error(format_args!("主键「{}」的值无法用来定位行。", column.name))
                    })?;
                    builder.sql(" = ");
                    builder.param((bound, self.cast(column)));
                }
            }
        }
        Ok(())
    }

    fn update(&self, key: &RowKey, values: &EditValues) -> CoreResult<Option<Statement>> {
        let assignments = self.writable(values)?;
        if assignments.is_empty() {
            return Ok(None);
        }
        let mut builder = Builder::new(self.dialect);
        builder.sql(&format!("UPDATE {} SET ", self.target));
        for (index, (column, input)) in assignments.into_iter().enumerate() {
            if index > 0 {
                builder.sql(", ");
            }
            builder.sql(&format!("{} = ", self.quote(column)));
            builder.param(self.value(column, input)?);
        }
        self.locate(&mut builder, key)?;
        Ok(Some(builder.finish()))
    }

    fn insert(&self, values: &EditValues) -> CoreResult<Statement> {
        let assignments = self.writable(values)?;
        let mut builder = Builder::new(self.dialect);
        builder.sql(&format!("INSERT INTO {}", self.target));
        if assignments.is_empty() {
            builder.sql(match self.dialect {
                Dialect::Mysql => " () VALUES ()",
                Dialect::Postgres | Dialect::Sqlite => " DEFAULT VALUES",
            });
            return Ok(builder.finish());
        }
        let names: Vec<String> = assignments.iter().map(|(c, _)| self.quote(c)).collect();
        builder.sql(&format!(" ({}) VALUES (", names.join(", ")));
        for (index, (column, input)) in assignments.into_iter().enumerate() {
            if index > 0 {
                builder.sql(", ");
            }
            builder.param(self.value(column, input)?);
        }
        builder.sql(")");
        Ok(builder.finish())
    }

    fn delete(&self, key: &RowKey) -> CoreResult<Statement> {
        let mut builder = Builder::new(self.dialect);
        builder.sql(&format!("DELETE FROM {}", self.target));
        self.locate(&mut builder, key)?;
        Ok(builder.finish())
    }
}

/// 把一批修改变成按「删除 → 更新 → 新增」排好序的语句:先删再插,同一主键
/// 先删除再新增时不会冲突。没有任何列要改的更新直接跳过。
pub fn build_changes(
    dialect: Dialect,
    table: &TableRef,
    columns: &[ColumnInfo],
    changes: &[RowChange],
) -> CoreResult<Vec<Statement>> {
    if let Some(reason) = read_only_reason(columns) {
        return Err(CoreError::ReadOnly(reason));
    }
    let target = table_name(dialect, table);
    let mut ordered: Vec<(usize, &RowChange)> = changes.iter().enumerate().collect();
    ordered.sort_by_key(|(_, change)| match change {
        RowChange::Delete { .. } => 0,
        RowChange::Update { .. } => 1,
        RowChange::Insert { .. } => 2,
    });
    let mut statements = Vec::with_capacity(changes.len());
    for (index, change) in ordered {
        let context = Change {
            dialect,
            target: &target,
            columns,
            number: index + 1,
        };
        let statement = match change {
            RowChange::Delete { key } => Some(context.delete(key)?),
            RowChange::Update { key, values } => context.update(key, values)?,
            RowChange::Insert { values } => Some(context.insert(values)?),
        };
        statements.extend(statement);
    }
    Ok(statements)
}

#[cfg(test)]
#[path = "dml_tests.rs"]
mod tests;
