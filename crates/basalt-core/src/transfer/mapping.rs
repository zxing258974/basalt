//! 导入的列映射:目标表的每一列取来源的哪一列(或者不导入)。

use serde::{Deserialize, Serialize};

use crate::table::ColumnInfo;

/// 目标列 `column` 取来源第 `source` 列;`source` 为 `None` 表示这一列不导入(用默认值)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMapping {
    pub column: String,
    pub source: Option<usize>,
}

/// 比较列名时忽略大小写、空白、下划线和连字符:「User Name」「user_name」「username」算同一个。
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// 按列名自动匹配:先找完全相同的,再找忽略大小写的,最后找规范化后相同的;
/// 每个来源列最多用一次。没有列名行时按位置一一对应。生成列不导入。
pub fn auto_map(
    headers: &[String],
    columns: &[ColumnInfo],
    has_header: bool,
) -> Vec<ColumnMapping> {
    let writable: Vec<&ColumnInfo> = columns.iter().filter(|c| !c.is_generated).collect();
    let mut sources: Vec<Option<usize>> = vec![None; writable.len()];
    let mut used = vec![false; headers.len()];
    if has_header {
        let rules: [fn(&str, &str) -> bool; 3] = [
            |a, b| a == b,
            |a, b| a.to_lowercase() == b.to_lowercase(),
            |a, b| normalize(a) == normalize(b),
        ];
        for rule in rules {
            for (slot, column) in sources.iter_mut().zip(&writable) {
                if slot.is_some() {
                    continue;
                }
                if let Some(index) =
                    (0..headers.len()).find(|&i| !used[i] && rule(&headers[i], &column.name))
                {
                    used[index] = true;
                    *slot = Some(index);
                }
            }
        }
    } else {
        for (index, slot) in sources.iter_mut().enumerate() {
            *slot = (index < headers.len()).then_some(index);
        }
    }
    writable
        .iter()
        .zip(sources)
        .map(|(column, source)| ColumnMapping {
            column: column.name.clone(),
            source,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn sources(mapping: &[ColumnMapping]) -> Vec<Option<usize>> {
        mapping.iter().map(|m| m.source).collect()
    }

    #[test]
    fn exact_names_win_over_looser_matches() {
        let columns = [
            ColumnInfo::new("id", "int"),
            ColumnInfo::new("user_name", "varchar"),
            ColumnInfo::new("Email", "varchar"),
            ColumnInfo::new("note", "text"),
        ];
        let mapping = auto_map(
            &headers(&["EMAIL", "User Name", "id", "其他"]),
            &columns,
            true,
        );
        assert_eq!(sources(&mapping), [Some(2), Some(1), Some(0), None]);
        assert_eq!(mapping[1].column, "user_name");
    }

    #[test]
    fn each_source_column_is_used_once() {
        let columns = [
            ColumnInfo::new("name", "text"),
            ColumnInfo::new("NAME", "text"),
        ];
        let mapping = auto_map(&headers(&["name"]), &columns, true);
        assert_eq!(sources(&mapping), [Some(0), None]);
    }

    #[test]
    fn generated_columns_are_left_out_and_position_is_used_without_header() {
        let columns = [
            ColumnInfo::new("a", "int"),
            ColumnInfo {
                is_generated: true,
                ..ColumnInfo::new("b", "int")
            },
            ColumnInfo::new("c", "int"),
            ColumnInfo::new("d", "int"),
        ];
        let mapping = auto_map(&headers(&["第 1 列", "第 2 列"]), &columns, false);
        let names: Vec<&str> = mapping.iter().map(|m| m.column.as_str()).collect();
        assert_eq!(names, ["a", "c", "d"]);
        assert_eq!(sources(&mapping), [Some(0), Some(1), None]);
    }
}
