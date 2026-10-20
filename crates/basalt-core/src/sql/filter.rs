//! 筛选框里的 WHERE 条件检查。
//!
//! 条件原样拼进 SELECT,只做词法检查:跳过字符串、带引号的标识符和注释后,
//! 不允许出现分号(SQLite 会把分号后的语句也执行掉),括号必须配对,
//! 引号和注释必须闭合。条件是用户写给自己的库的,不做语义限制。

use super::Dialect;

const SEMICOLON: &str = "筛选条件里不能有分号：这里只填 WHERE 后面的条件，不能再接别的语句。";
const UNBALANCED: &str = "筛选条件里的括号不配对。";
const UNTERMINATED: &str = "筛选条件里有引号或注释没有闭合。";

/// 检查通过返回 `Ok`,否则返回给用户看的原因。
pub fn check_filter(dialect: Dialect, filter: &str) -> Result<(), String> {
    let chars: Vec<char> = filter.chars().collect();
    let mut depth = 0_i32;
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = skip_literal_or_comment(dialect, &chars, i)? {
            i = end;
            continue;
        }
        match chars[i] {
            ';' => return Err(SEMICOLON.into()),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(UNBALANCED.into());
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth == 0 {
        Ok(())
    } else {
        Err(UNBALANCED.into())
    }
}

/// `i` 处如果开始一个字符串、带引号的标识符或注释,返回它结束后的位置。
fn skip_literal_or_comment(
    dialect: Dialect,
    chars: &[char],
    i: usize,
) -> Result<Option<usize>, String> {
    let next = chars.get(i + 1).copied();
    let end = match (chars[i], dialect) {
        ('\'', _) => skip_quoted(chars, i, '\'', backslash_escapes(dialect, chars, i))?,
        // MySQL 的双引号是字符串,同样认反斜杠转义;另外两种是标识符。
        ('"', _) => skip_quoted(chars, i, '"', dialect == Dialect::Mysql)?,
        ('`', Dialect::Mysql | Dialect::Sqlite) => skip_quoted(chars, i, '`', false)?,
        ('[', Dialect::Sqlite) => find(chars, i + 1, &[']']).ok_or(UNTERMINATED)? + 1,
        ('$', Dialect::Postgres) => match dollar_tag(chars, i) {
            Some(tag) => {
                let body = i + tag.len();
                find(chars, body, &tag).ok_or(UNTERMINATED)? + tag.len()
            }
            None => return Ok(None),
        },
        ('-', _) if next == Some('-') && line_comment_starts(dialect, chars, i) => {
            find(chars, i, &['\n']).unwrap_or(chars.len())
        }
        ('#', Dialect::Mysql) => find(chars, i, &['\n']).unwrap_or(chars.len()),
        ('/', _) if next == Some('*') => {
            skip_block_comment(chars, i, dialect == Dialect::Postgres)?
        }
        _ => return Ok(None),
    };
    Ok(Some(end))
}

/// MySQL 默认认反斜杠转义;PostgreSQL 只有 E'...' 才认。
fn backslash_escapes(dialect: Dialect, chars: &[char], quote: usize) -> bool {
    match dialect {
        Dialect::Mysql => true,
        Dialect::Postgres => {
            let prefix = quote.checked_sub(1).map(|p| chars[p]);
            let before_prefix = quote.checked_sub(2).map(|p| chars[p]);
            matches!(prefix, Some('E' | 'e')) && !before_prefix.is_some_and(is_ident_char)
        }
        Dialect::Sqlite => false,
    }
}

/// MySQL 的 `--` 后面必须跟空白才算注释,否则 `1--1` 是减负一。
fn line_comment_starts(dialect: Dialect, chars: &[char], i: usize) -> bool {
    dialect != Dialect::Mysql || chars.get(i + 2).is_none_or(|c| c.is_whitespace())
}

/// 从 `start` 的开引号起,跳到配对的闭引号之后。成对的引号是转义。
fn skip_quoted(
    chars: &[char],
    start: usize,
    quote: char,
    backslash: bool,
) -> Result<usize, String> {
    let mut j = start + 1;
    while j < chars.len() {
        match chars[j] {
            '\\' if backslash => j += 2,
            c if c == quote => {
                if chars.get(j + 1) == Some(&quote) {
                    j += 2;
                } else {
                    return Ok(j + 1);
                }
            }
            _ => j += 1,
        }
    }
    Err(UNTERMINATED.into())
}

/// PostgreSQL 的块注释可以嵌套,MySQL 与 SQLite 不行。
fn skip_block_comment(chars: &[char], start: usize, nested: bool) -> Result<usize, String> {
    let mut depth = 0;
    let mut j = start;
    while j + 1 < chars.len() {
        match (chars[j], chars[j + 1]) {
            ('/', '*') if depth == 0 || nested => {
                depth += 1;
                j += 2;
            }
            ('*', '/') => {
                depth -= 1;
                j += 2;
                if depth == 0 {
                    return Ok(j);
                }
            }
            _ => j += 1,
        }
    }
    Err(UNTERMINATED.into())
}

/// `$tag$` 形式的美元引号开头,返回包括两端 `$` 的标签。`$1` 是参数,
/// 标识符中间的 `$` 也不算。
fn dollar_tag(chars: &[char], start: usize) -> Option<Vec<char>> {
    if start > 0 && is_ident_char(chars[start - 1]) {
        return None;
    }
    let close = find(chars, start + 1, &['$'])?;
    let tag = &chars[start + 1..close];
    let valid =
        tag.first().is_none_or(|c| !c.is_ascii_digit()) && tag.iter().all(|c| is_ident_char(*c));
    valid.then(|| chars[start..=close].to_vec())
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// 从 `from` 起找子序列 `needle` 第一次出现的位置。
fn find(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if from > chars.len() || needle.is_empty() {
        return None;
    }
    chars[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Dialect::{Mysql, Postgres, Sqlite};

    fn ok(dialect: Dialect, filter: &str) {
        assert_eq!(
            check_filter(dialect, filter),
            Ok(()),
            "{dialect:?}: {filter}"
        );
    }

    fn rejected(dialect: Dialect, filter: &str, reason: &str) {
        let error = check_filter(dialect, filter).expect_err(filter);
        assert!(error.contains(reason), "{dialect:?}: {filter} → {error}");
    }

    #[test]
    fn plain_conditions_pass() {
        for dialect in [Mysql, Postgres, Sqlite] {
            ok(dialect, "age > 30 AND (name LIKE 'a%' OR name IS NULL)");
        }
    }

    #[test]
    fn semicolons_inside_strings_identifiers_and_comments_are_fine() {
        ok(
            Postgres,
            "note = 'a;b' AND \"we;ird\" = 1 -- trailing; comment",
        );
        ok(Postgres, "body = $$x; y$$ AND t = $tag$;$tag$");
        ok(Postgres, "a = 1 /* outer; /* nested; */ still; */");
        ok(Mysql, "note = 'it\\'s; ok' AND `a;b` = 1 # tail;");
        ok(Mysql, "note = \"dq; string\" -- tail;");
        ok(Sqlite, "[odd;name] = 1 AND `b;` = 2 AND \"c;\" = 3");
    }

    #[test]
    fn a_second_statement_is_rejected() {
        for dialect in [Mysql, Postgres, Sqlite] {
            rejected(dialect, "1 = 1; DELETE FROM t", "分号");
            rejected(dialect, "id = 1;", "分号");
        }
    }

    #[test]
    fn parentheses_must_balance() {
        rejected(Postgres, "a = (1", "括号");
        rejected(Mysql, "a = 1) OR (b = 2", "括号");
        ok(Mysql, "a = ')'");
    }

    #[test]
    fn quotes_and_comments_must_be_closed() {
        rejected(Postgres, "name = 'abc", "没有闭合");
        rejected(Sqlite, "x = 1 /* open", "没有闭合");
        rejected(Mysql, "`col = 1", "没有闭合");
        rejected(Postgres, "a = $q$never", "没有闭合");
    }

    #[test]
    fn mysql_double_dash_is_a_comment_only_before_whitespace() {
        // MySQL 里 1--1 是减负一,分号仍在注释外面。
        rejected(Mysql, "a = 1--1; DROP TABLE t", "分号");
        ok(Postgres, "a = 1--1; DROP TABLE t");
        ok(Mysql, "a = 1 -- ;\n AND b = 2");
    }

    #[test]
    fn backslash_escapes_only_in_mysql_strings_and_postgres_e_strings() {
        // PostgreSQL 的普通字符串里反斜杠就是字符本身。
        ok(Postgres, "path = 'C:\\'");
        rejected(Mysql, "path = 'C:\\'", "没有闭合");
        ok(Postgres, "x = E'it\\'s'");
        ok(Sqlite, "path = 'C:\\'");
    }

    #[test]
    fn doubled_quotes_stay_inside_the_string() {
        ok(Postgres, "name = 'O''Brien; Jr'");
        ok(Sqlite, "\"a\"\"b;\" = 1");
    }
}
