//! SQLite 没有 `CREATE TABLE … LIKE`:复制表时把 `sqlite_master` 里存的建表、建索引语句
//! 换上新名字再执行,约束、排序规则、WITHOUT ROWID / STRICT 原样保留。
//!
//! SQLite 存进去的语句做过规范化:去掉了开头的空白、`IF NOT EXISTS`、TEMP 和库名限定,
//! 头两个关键字后面是单个空格(<https://www.sqlite.org/schematab.html>)。
//! 这里只认出开头的关键字和名字,后面的部分一个字不改。

use crate::error::{CoreError, CoreResult};

/// 从 `text` 开头读一个标识符(可能加了 `"…"`、`` `…` ``、`[…]` 或 `'…'`),返回它的原文长度。
fn identifier_len(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    let close = match first {
        '"' => '"',
        '`' => '`',
        '\'' => '\'',
        '[' => ']',
        c if c.is_alphanumeric() || c == '_' || c == '$' || !c.is_ascii() => {
            let end = text
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$' || !c.is_ascii()))
                .map_or(text.len(), |(i, _)| i);
            return Some(end);
        }
        _ => return None,
    };
    let mut chars = chars.peekable();
    while let Some((index, c)) = chars.next() {
        if c != close {
            continue;
        }
        // 引号里连写两个表示一个字面的引号;方括号没有转义。
        if close != ']' && chars.peek().is_some_and(|(_, next)| *next == close) {
            chars.next();
            continue;
        }
        return Some(index + c.len_utf8());
    }
    None
}

/// 跳过空白,要求接下来是关键字 `keyword`(不分大小写),返回剩下的部分。
fn keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let text = text.trim_start();
    let head = text.get(..keyword.len())?;
    let after = &text[keyword.len()..];
    let boundary = after
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    (head.eq_ignore_ascii_case(keyword) && boundary).then_some(after)
}

fn unsupported(what: &str) -> CoreError {
    CoreError::Unsupported(format!(
        "认不出这张表的{what}，没法复制。可以在 SQL 编辑器里手动建表。"
    ))
}

/// `CREATE TABLE 旧名 …` → `CREATE TABLE 新名 …`,`new_name` 已经加好引号(可带库名)。虚拟表(FTS 等)不支持。
pub(crate) fn rename_table_sql(sql: &str, new_name: &str) -> CoreResult<String> {
    let rest = keyword(sql, "CREATE").ok_or_else(|| unsupported("建表语句"))?;
    if keyword(rest, "VIRTUAL").is_some() {
        return Err(CoreError::Unsupported(
            "虚拟表（如全文检索 FTS）不能复制。".into(),
        ));
    }
    let rest = keyword(rest, "TABLE")
        .ok_or_else(|| unsupported("建表语句"))?
        .trim_start();
    let len = identifier_len(rest).ok_or_else(|| unsupported("建表语句"))?;
    Ok(format!("CREATE TABLE {new_name}{}", &rest[len..]))
}

/// 新索引的名字:旧名里有旧表名就换成新表名,没有就在前面加上新表名。
/// SQLite 的索引名在整个库里不能重复,不能照搬。
pub(crate) fn index_name(index: &str, old_table: &str, new_table: &str) -> String {
    // 只换按下划线分开的完整单词:表 a 的索引 idx_name_a 该变成 idx_name_b,不能变成 idx_nbme_a。
    let starts = |i: usize| i == 0 || index[..i].ends_with('_');
    let ends = |j: usize| j == index.len() || index[j..].starts_with('_');
    let whole = (!old_table.is_empty())
        .then(|| {
            index
                .match_indices(old_table)
                .map(|(i, _)| i)
                .find(|&i| starts(i) && ends(i + old_table.len()))
        })
        .flatten();
    match whole {
        Some(i) => format!(
            "{}{new_table}{}",
            &index[..i],
            &index[i + old_table.len()..]
        ),
        None => format!("{new_table}_{index}"),
    }
}

/// `CREATE [UNIQUE] INDEX 旧索引 ON 旧表 …` → 换成新索引名和新表名,列和 WHERE 条件不变。
/// 两个名字都已经加好引号;索引名可带库名,`ON` 后面的表名不能带。
pub(crate) fn rename_index_sql(sql: &str, new_index: &str, new_table: &str) -> CoreResult<String> {
    let after_create = keyword(sql, "CREATE").ok_or_else(|| unsupported("索引"))?;
    let (unique, rest) = match keyword(after_create, "UNIQUE") {
        Some(rest) => (true, rest),
        None => (false, after_create),
    };
    let rest = keyword(rest, "INDEX")
        .ok_or_else(|| unsupported("索引"))?
        .trim_start();
    let len = identifier_len(rest).ok_or_else(|| unsupported("索引"))?;
    let rest = keyword(&rest[len..], "ON")
        .ok_or_else(|| unsupported("索引"))?
        .trim_start();
    let len = identifier_len(rest).ok_or_else(|| unsupported("索引"))?;
    Ok(format!(
        "CREATE {}INDEX {new_index} ON {new_table}{}",
        if unique { "UNIQUE " } else { "" },
        &rest[len..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_names_in_every_quoting_style_are_replaced() {
        for (sql, rest) in [
            ("CREATE TABLE people (id INTEGER)", " (id INTEGER)"),
            ("CREATE TABLE \"my \"\"people\"\"\"(id)", "(id)"),
            ("CREATE TABLE `people`(id)", "(id)"),
            (
                "CREATE TABLE [my people] (id) WITHOUT ROWID",
                " (id) WITHOUT ROWID",
            ),
            ("create table 用户表(id) STRICT", "(id) STRICT"),
        ] {
            assert_eq!(
                rename_table_sql(sql, "\"main\".\"people_copy1\"").unwrap(),
                format!("CREATE TABLE \"main\".\"people_copy1\"{rest}"),
                "{sql}"
            );
        }
    }

    #[test]
    fn virtual_tables_and_unknown_shapes_are_refused() {
        let error =
            rename_table_sql("CREATE VIRTUAL TABLE docs USING fts5(body)", "x").unwrap_err();
        assert!(error.to_string().contains("虚拟表"));
        assert!(rename_table_sql("CREATE TABLE \"unterminated(a)", "x").is_err());
        assert!(rename_table_sql("SELECT 1", "x").is_err());
    }

    #[test]
    fn indexes_get_a_new_name_and_point_at_the_new_table() {
        assert_eq!(
            rename_index_sql(
                "CREATE UNIQUE INDEX  people_code ON \"people\" (code COLLATE NOCASE) WHERE code IS NOT NULL",
                "\"people_copy1_code\"",
                "\"people_copy1\""
            )
            .unwrap(),
            "CREATE UNIQUE INDEX \"people_copy1_code\" ON \"people_copy1\" (code COLLATE NOCASE) WHERE code IS NOT NULL"
        );
        assert_eq!(
            rename_index_sql("CREATE INDEX [ix] on people(a)", "\"aux\".\"n\"", "\"t\"").unwrap(),
            "CREATE INDEX \"aux\".\"n\" ON \"t\"(a)"
        );
        // 表名恰好以 ON 开头,不能把它当成关键字 ON。
        assert!(rename_index_sql("CREATE INDEX ix ONx(a)", "n", "t").is_err());
    }

    #[test]
    fn index_names_follow_the_new_table() {
        assert_eq!(
            index_name("people_code", "people", "people_copy1"),
            "people_copy1_code"
        );
        assert_eq!(index_name("idx_people_code", "people", "p2"), "idx_p2_code");
        assert_eq!(index_name("ix_code", "people", "p2"), "p2_ix_code");
        assert_eq!(index_name("idx_name_a", "a", "b"), "idx_name_b");
        assert_eq!(index_name("peoplex_idx", "people", "p2"), "p2_peoplex_idx");
        assert_eq!(index_name("a", "a", "b"), "b");
    }
}
