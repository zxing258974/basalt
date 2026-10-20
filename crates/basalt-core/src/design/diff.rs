//! 原结构与新结构按 id 配对后的差异,三种方言共用。

use std::collections::HashSet;

use super::validate::name_key;
use super::{ColumnDef, ForeignKeyDef, IndexDef, TableDesign};
use crate::sql::Dialect;

/// 新结构里的一个字段;原有的字段带着原定义。
pub(super) struct TargetColumn<'a> {
    pub after: &'a ColumnDef,
    pub before: Option<&'a ColumnDef>,
}

impl TargetColumn<'_> {
    pub fn renamed(&self) -> bool {
        self.before.is_some_and(|b| b.name != self.after.name)
    }

    /// 除名字以外的定义变了。
    pub fn changed(&self) -> bool {
        self.before.is_some_and(|b| !same_definition(b, self.after))
    }
}

pub(super) fn same_definition(a: &ColumnDef, b: &ColumnDef) -> bool {
    a.data_type == b.data_type
        && a.nullable == b.nullable
        && a.default == b.default
        && a.auto_increment == b.auto_increment
        && a.comment == b.comment
        && a.collation == b.collation
        && a.on_update == b.on_update
        && a.generated == b.generated
}

/// 改名、改定义都算一种「变了」;只改名的另记,有的方言能直接改名。
pub(super) struct Changes<'a, T> {
    /// 删掉的,以及定义变了要先删再建的(原定义)。
    pub dropped: Vec<&'a T>,
    /// 只改了名字的:(原定义, 新定义)。
    pub renamed: Vec<(&'a T, &'a T)>,
    /// 新加的,以及定义变了要重建的(新定义)。
    pub added: Vec<&'a T>,
}

impl<T> Changes<'_, T> {
    pub fn is_empty(&self) -> bool {
        self.dropped.is_empty() && self.renamed.is_empty() && self.added.is_empty()
    }
}

pub(super) struct Diff<'a> {
    pub dialect: Dialect,
    pub original: &'a TableDesign,
    pub target: &'a TableDesign,
    /// 新结构的字段,按新顺序。
    pub columns: Vec<TargetColumn<'a>>,
    pub dropped_columns: Vec<&'a ColumnDef>,
    pub primary_key_changed: bool,
    pub indexes: Changes<'a, IndexDef>,
    pub foreign_keys: Changes<'a, ForeignKeyDef>,
}

impl<'a> Diff<'a> {
    pub fn new(dialect: Dialect, original: &'a TableDesign, target: &'a TableDesign) -> Self {
        let columns = target
            .columns
            .iter()
            .map(|after| TargetColumn {
                after,
                before: original.columns.iter().find(|b| b.id == after.id),
            })
            .collect();
        let dropped_columns = original
            .columns
            .iter()
            .filter(|b| !target.columns.iter().any(|a| a.id == b.id))
            .collect();
        Self {
            dialect,
            original,
            target,
            columns,
            dropped_columns,
            primary_key_changed: original.primary_key != target.primary_key,
            indexes: changes(
                &original.indexes,
                &target.indexes,
                |i| &i.id,
                |i| &i.name,
                |a, b| {
                    a.columns == b.columns
                        && a.unique == b.unique
                        && a.method == b.method
                        && a.definition == b.definition
                },
            ),
            foreign_keys: changes(
                &original.foreign_keys,
                &target.foreign_keys,
                |f| &f.id,
                |f| &f.name,
                |a, b| {
                    a.columns == b.columns
                        && ref_schema(a, original) == ref_schema(b, target)
                        && a.ref_table == b.ref_table
                        && a.ref_columns == b.ref_columns
                        && a.on_update == b.on_update
                        && a.on_delete == b.on_delete
                },
            ),
        }
    }

    pub fn table_renamed(&self) -> bool {
        self.original.table.table != self.target.table.table
    }

    /// 原有字段在新顺序里的原位置。
    fn kept_positions(&self) -> Vec<usize> {
        self.columns
            .iter()
            .filter_map(|c| {
                c.before
                    .and_then(|b| self.original.columns.iter().position(|o| o.id == b.id))
            })
            .collect()
    }

    /// 原有字段之间的先后次序变了。
    pub fn existing_reordered(&self) -> bool {
        self.kept_positions().windows(2).any(|w| w[0] > w[1])
    }

    /// 有新字段排在原有字段前面(不能简单地追加在最后)。
    pub fn added_before_existing(&self) -> bool {
        let first_new = self.columns.iter().position(|c| c.before.is_none());
        let last_kept = self.columns.iter().rposition(|c| c.before.is_some());
        matches!((first_new, last_kept), (Some(new), Some(kept)) if new < kept)
    }

    /// 每个新字段是否不用挪动:只挪最长递增子序列以外的原有字段。按新顺序排列,新字段一律为 false。
    pub fn stable_columns(&self) -> Vec<bool> {
        let positions = self.kept_positions();
        let in_lis = longest_increasing(&positions);
        let mut kept = in_lis.into_iter();
        self.columns
            .iter()
            .map(|c| c.before.is_some() && kept.next().unwrap_or(false))
            .collect()
    }

    /// 字段改名的步骤 (原名, 新名)。新名和别的字段现有的名字撞上时(比如两个字段互换名字),
    /// 先全部改成临时名再改回来。
    pub fn rename_steps(&self) -> Vec<(String, String)> {
        let renames: Vec<(&str, &str)> = self
            .columns
            .iter()
            .filter(|c| c.renamed())
            .filter_map(|c| c.before.map(|b| (b.name.as_str(), c.after.name.as_str())))
            .collect();
        let existing: HashSet<String> = self
            .columns
            .iter()
            .filter_map(|c| c.before)
            .map(|b| name_key(self.dialect, &b.name))
            .collect();
        let conflict = renames.iter().any(|(from, to)| {
            let to = name_key(self.dialect, to);
            to != name_key(self.dialect, from) && existing.contains(&to)
        });
        if !conflict {
            return renames
                .into_iter()
                .map(|(from, to)| (from.to_owned(), to.to_owned()))
                .collect();
        }
        let temporary: Vec<String> = (1..=renames.len())
            .map(|n| format!("__basalt_tmp_{n}"))
            .collect();
        let first = renames
            .iter()
            .zip(&temporary)
            .map(|((from, _), tmp)| ((*from).to_owned(), tmp.clone()));
        let second = renames
            .iter()
            .zip(&temporary)
            .map(|((_, to), tmp)| (tmp.clone(), (*to).to_owned()));
        first.chain(second).collect()
    }
}

/// 被引用表的 schema 写全:`None` 就是本表所在的库或 schema。
pub(super) fn ref_schema<'a>(fk: &'a ForeignKeyDef, own: &'a TableDesign) -> &'a str {
    match &fk.ref_schema {
        Some(schema) => schema,
        None => own.table.schema.as_deref().unwrap_or(&own.table.database),
    }
}

fn changes<'a, T>(
    before: &'a [T],
    after: &'a [T],
    id: impl Fn(&T) -> &String,
    name: impl Fn(&T) -> &String,
    same: impl Fn(&T, &T) -> bool,
) -> Changes<'a, T> {
    let mut result = Changes {
        dropped: Vec::new(),
        renamed: Vec::new(),
        added: Vec::new(),
    };
    for old in before {
        match after.iter().find(|new| id(new) == id(old)) {
            None => result.dropped.push(old),
            Some(new) if !same(old, new) => result.dropped.push(old),
            Some(new) if name(old) != name(new) => result.renamed.push((old, new)),
            Some(_) => {}
        }
    }
    for new in after {
        match before.iter().find(|old| id(old) == id(new)) {
            Some(old) if same(old, new) => {}
            _ => result.added.push(new),
        }
    }
    result
}

/// 最长递增子序列里的元素标 true;等长时取靠前的。
fn longest_increasing(values: &[usize]) -> Vec<bool> {
    let n = values.len();
    let mut length = vec![1_usize; n];
    let mut previous = vec![None; n];
    for i in 0..n {
        for j in 0..i {
            if values[j] < values[i] && length[j] + 1 > length[i] {
                length[i] = length[j] + 1;
                previous[i] = Some(j);
            }
        }
    }
    let mut marks = vec![false; n];
    let best = (0..n).max_by_key(|&i| (length[i], std::cmp::Reverse(i)));
    let mut cursor = best;
    while let Some(i) = cursor {
        marks[i] = true;
        cursor = previous[i];
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::longest_increasing;

    #[test]
    fn longest_increasing_marks_the_kept_run() {
        assert_eq!(longest_increasing(&[0, 3, 1, 2]), [true, false, true, true]);
        assert_eq!(longest_increasing(&[]), Vec::<bool>::new());
        assert_eq!(longest_increasing(&[2, 1, 0]), [true, false, false]);
    }
}
