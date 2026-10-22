//! 把驱动的 `fetch_many` 流收成结果集。三种驱动的流形状一样:若干行之后跟一个「完成」,
//! 一条语句可以有多个结果集;没有行的「完成」带的是增删改的影响行数。

use futures_util::{Stream, StreamExt};
use sqlx::{Column, Either, Row, Statement, TypeInfo};

use super::{MAX_RESULT_ROWS, ResultColumn, ResultSet};
use crate::value::CellValue;

pub(crate) struct Collected {
    pub result_sets: Vec<ResultSet>,
    /// 不带行的「完成」里的影响行数之和。
    pub affected: u64,
    /// 中途出错(含被取消)时,之前读到的结果照样保留。
    pub error: Option<sqlx::Error>,
}

pub(crate) fn columns_of<R: Row>(row: &R) -> Vec<ResultColumn> {
    row.columns()
        .iter()
        .map(|column| ResultColumn {
            name: column.name().to_owned(),
            data_type: type_label(column.type_info().name()),
        })
        .collect()
}

/// 没读到行的查询:预备一下(不执行)拿到的列名。预备不了就当没有结果集。
pub(crate) fn described_columns<S: Statement>(
    prepared: Result<S, sqlx::Error>,
) -> Vec<ResultColumn> {
    match prepared {
        Ok(statement) => statement
            .columns()
            .iter()
            .map(|column| ResultColumn {
                name: column.name().to_owned(),
                data_type: type_label(column.type_info().name()),
            })
            .collect(),
        Err(error) => {
            log::debug!("取空结果的列名失败：{error}");
            Vec::new()
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
    let mut current: Option<ResultSet> = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(Either::Right(row)) => {
                let set = current.get_or_insert_with(|| ResultSet {
                    columns: columns_of(&row),
                    rows: Vec::new(),
                    truncated: false,
                });
                if set.rows.len() < MAX_RESULT_ROWS {
                    set.rows
                        .push((0..set.columns.len()).map(|i| decode(&row, i)).collect());
                } else {
                    set.truncated = true;
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
