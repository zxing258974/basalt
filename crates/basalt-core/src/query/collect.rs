//! 把驱动的 `fetch_many` 流收成结果集。三种驱动的流形状一样:若干行之后跟一个「完成」,
//! 一条语句可以有多个结果集;没有行的「完成」带的是增删改的影响行数。

use futures_util::{Stream, StreamExt};
use sqlx::{Column, Database, Either, Row, Statement, TypeInfo};

use super::ResultColumn;
use super::results::{budget, row_bytes};
use crate::value::CellValue;

/// 驱动报的「这一列来自哪里」,还没换成 [`super::ColumnSource`]。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RawOrigin {
    None,
    /// MySQL、SQLite:驱动给的表名(带库名时是「库.表」)和原来的列名。
    Named {
        table: String,
        column: String,
    },
    /// PostgreSQL:表的 OID 和列号,执行完再查系统目录换成名字。
    Relation {
        oid: u32,
        attnum: i16,
    },
}

/// 一个结果集的全部行(超出内存预算的除外),还没存进 [`super::results::ResultStore`]。
pub(crate) struct CollectedSet {
    pub columns: Vec<ResultColumn>,
    pub origins: Vec<RawOrigin>,
    pub rows: Vec<Vec<CellValue>>,
    /// 存下的行估算占多少内存。
    pub bytes: usize,
    /// 超出内存预算(见 [`budget`]),后面的行没有存。
    pub truncated: bool,
}

pub(crate) struct Collected {
    pub result_sets: Vec<CollectedSet>,
    /// 不带行的「完成」里的影响行数之和。
    pub affected: u64,
    /// 中途出错(含被取消)时,之前读到的结果照样保留。
    pub error: Option<sqlx::Error>,
}

/// 各驱动从列上读来源的函数。
pub(crate) type OriginOf<DB> = fn(&<DB as Database>::Column) -> RawOrigin;

/// MySQL、SQLite 的驱动直接给出来源。
pub(crate) fn named_origin<C: Column>(column: &C) -> RawOrigin {
    match column.origin() {
        sqlx::ColumnOrigin::Table(source) => RawOrigin::Named {
            table: source.table.to_string(),
            column: source.name.to_string(),
        },
        _ => RawOrigin::None,
    }
}

fn result_column<C: Column>(column: &C) -> ResultColumn {
    ResultColumn {
        name: column.name().to_owned(),
        data_type: type_label(column.type_info().name()),
        source: None,
    }
}

pub(crate) fn columns_of<R: Row>(row: &R) -> Vec<ResultColumn> {
    row.columns().iter().map(result_column).collect()
}

/// 没读到行的查询:预备一下(不执行)拿到的列名和来源。预备不了就当没有结果集。
pub(crate) fn described_set<S: Statement>(
    prepared: Result<S, sqlx::Error>,
    origin_of: OriginOf<S::Database>,
) -> Option<CollectedSet> {
    match prepared {
        Ok(statement) if !statement.columns().is_empty() => Some(CollectedSet {
            columns: statement.columns().iter().map(result_column).collect(),
            origins: statement.columns().iter().map(origin_of).collect(),
            rows: Vec::new(),
            bytes: 0,
            truncated: false,
        }),
        Ok(_) => None,
        Err(error) => {
            log::debug!("取空结果的列名失败：{error}");
            None
        }
    }
}

/// 驱动的类型名转小写给人看;sqlx 认不出的类型报成 `?`,SQLite 表达式列报成 NULL,都显示为空。
pub(crate) fn type_label(name: &str) -> String {
    if name == "?" || name.eq_ignore_ascii_case("null") {
        String::new()
    } else {
        name.to_ascii_lowercase()
    }
}

/// 解码函数用函数指针而不是泛型闭包:泛型闭包的高阶生命周期会让外面的 future
/// 被编译器判成「不是对所有生命周期都 Send」,Tauri 命令和 tokio::spawn 都用不了。
pub(crate) async fn collect<S, Q, R>(
    mut stream: S,
    decode: fn(&R, usize) -> CellValue,
    origin_of: OriginOf<R::Database>,
    affected_of: fn(&Q) -> u64,
) -> Collected
where
    S: Stream<Item = Result<Either<Q, R>, sqlx::Error>> + Unpin,
    R: Row,
{
    let mut collected = Collected {
        result_sets: Vec::new(),
        affected: 0,
        error: None,
    };
    let mut current: Option<CollectedSet> = None;
    // 一条语句的所有结果集合起来算预算;别的标签页存得多时,这里能存的就少。
    let limit = budget();
    let mut used = 0usize;
    while let Some(item) = stream.next().await {
        match item {
            Ok(Either::Right(row)) => {
                let set = current.get_or_insert_with(|| CollectedSet {
                    columns: columns_of(&row),
                    origins: row.columns().iter().map(origin_of).collect(),
                    rows: Vec::new(),
                    bytes: 0,
                    truncated: false,
                });
                if set.truncated {
                    continue;
                }
                let values: Vec<CellValue> =
                    (0..set.columns.len()).map(|i| decode(&row, i)).collect();
                let size = row_bytes(&values);
                used += size;
                if used > limit {
                    set.truncated = true;
                } else {
                    set.bytes += size;
                    set.rows.push(values);
                }
            }
            Ok(Either::Left(done)) => match current.take() {
                Some(set) => collected.result_sets.push(set),
                None => collected.affected += affected_of(&done),
            },
            Err(error) => {
                collected.error = Some(error);
                break;
            }
        }
    }
    collected.result_sets.extend(current);
    collected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_type_names_are_blank() {
        assert_eq!(type_label("?"), "");
        assert_eq!(type_label("NULL"), "");
        assert_eq!(type_label("VARCHAR"), "varchar");
        assert_eq!(type_label("INT4"), "int4");
    }
}
