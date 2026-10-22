//! 语句错在哪里:换成语句里的第几个 UTF-16 单元(前端编辑器按它数位置),编辑器据此标出来。
//!
//! PostgreSQL 直接给位置(第几个字符,从 1 数);MySQL / MariaDB 的语法错误说「near '…' at line N」,
//! 那段原文就是出错处往后的部分(太长会截短):语句以它结尾就是它,否则按那一行往后找;
//! 别的错误(MySQL 的未知列、不存在的表、函数,SQLite 的 near "…"、no such …)找报错里提到的名字,
//! 不算注释和字符串里的,而且只有出现一次时才标:标错地方不如不标。认不出时为 None。

use sqlx::mysql::MySqlDatabaseError;
use sqlx::postgres::{PgDatabaseError, PgErrorPosition};

pub(crate) fn error_position(error: &sqlx::Error, sql: &str) -> Option<usize> {
    let sqlx::Error::Database(db) = error else {
        return None;
    };
    let byte = if let Some(pg) = db.try_downcast_ref::<PgDatabaseError>() {
        // Internal 是函数体里生成的语句的位置,不在用户写的这条里。
        match pg.position()? {
            PgErrorPosition::Original(at) => char_byte(sql, at.checked_sub(1)?)?,
            PgErrorPosition::Internal { .. } => return None,
        }
    } else if db.try_downcast_ref::<MySqlDatabaseError>().is_some() {
        mysql_position(db.message(), sql)?
    } else {
        sqlite_position(db.message(), sql)?
    };
    Some(sql[..byte].encode_utf16().count())
}

/// 第 `index` 个字符(从 0 数)的字节位置;正好在末尾时是长度。
fn char_byte(sql: &str, index: usize) -> Option<usize> {
    sql.char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(sql.len()))
        .nth(index)
}

/// 第 `line` 行(从 1 数)开头的字节位置。
fn line_start(sql: &str, line: usize) -> Option<usize> {
    if line <= 1 {
        return Some(0);
    }
    sql.match_indices('\n').nth(line - 2).map(|(at, _)| at + 1)
}

/// 报错里 `prefix` 后面、到下一个 `close` 为止的那段。
fn quoted_after<'a>(message: &'a str, prefix: &str, close: char) -> Option<&'a str> {
    let rest = &message[message.find(prefix)? + prefix.len()..];
    Some(&rest[..rest.find(close)?])
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// 注释和字符串的字节范围,里面出现的名字不算。`double_quoted_strings`:MySQL 里 "…" 是字符串,
/// SQLite 里是标识符(可能正是要找的名字)。
fn literal_ranges(sql: &str, double_quoted_strings: bool) -> Vec<(usize, usize)> {
    let bytes = sql.as_bytes();
    let mut ranges = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        let end = match bytes[at] {
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                sql[at..].find('\n').map_or(sql.len(), |n| at + n)
            }
            b'#' if double_quoted_strings => sql[at..].find('\n').map_or(sql.len(), |n| at + n),
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                sql[at + 2..].find("*/").map_or(sql.len(), |n| at + n + 4)
            }
            quote @ (b'\'' | b'"') if quote == b'\'' || double_quoted_strings => {
                let mut end = at + 1;
                while end < bytes.len() {
                    match bytes[end] {
                        b'\\' => end += 2,
                        c if c == quote && bytes.get(end + 1) == Some(&quote) => end += 2,
                        c if c == quote => break,
                        _ => end += 1,
                    }
                }
                (end + 1).min(bytes.len())
            }
            _ => {
                at += 1;
                continue;
            }
        };
        ranges.push((start, end));
        at = end.max(start + 1);
    }
    ranges
}

/// `name` 作为一个完整的词(前后不是字母数字下划线)出现在注释、字符串以外的地方,而且只出现一次时的位置;
/// 带限定名(`o.total`、`shop.missing`)找不到时再按最后一段找。
fn find_name(sql: &str, name: &str, double_quoted_strings: bool) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    let literals = literal_ranges(sql, double_quoted_strings);
    let unique = |needle: &str| {
        let mut found = sql.match_indices(needle).map(|(at, _)| at).filter(|&at| {
            let before = sql[..at].chars().next_back();
            let after = sql[at + needle.len()..].chars().next();
            !before.is_some_and(is_word)
                && !after.is_some_and(is_word)
                && !literals.iter().any(|&(from, to)| from <= at && at < to)
        });
        match (found.next(), found.next()) {
            (Some(at), None) => Some(Some(at)),
            (Some(_), Some(_)) => Some(None),
            (None, _) => None,
        }
    };
    // 整个名字找到了(哪怕不止一处)就不再按最后一段找。
    match unique(name) {
        Some(found) => found,
        None => {
            let last = name.rsplit('.').next().filter(|last| *last != name)?;
            unique(last).flatten()
        }
    }
}

/// 语句末尾(去掉结尾的空白):「语句不完整」就标在这里。
fn end_of(sql: &str) -> usize {
    sql.trim_end().len()
}

fn mysql_position(message: &str, sql: &str) -> Option<usize> {
    // 「… right syntax to use near 'FORM t' at line 1」:那段原文从出错的地方开始(可能被截短)。
    const NEAR: &str = "near '";
    const AT_LINE: &str = "' at line ";
    if let Some(at) = message.rfind(AT_LINE) {
        let start = message.find(NEAR)? + NEAR.len();
        let snippet = message.get(start..at)?;
        let line: usize = message[at + AT_LINE.len()..].trim().parse().ok()?;
        if snippet.is_empty() {
            return Some(end_of(sql));
        }
        // 没截短的话,那段原文就是语句的结尾部分:这是最准的。
        if let Some(head) = sql.trim_end().strip_suffix(snippet) {
            return Some(head.len());
        }
        // 报的行号不总可靠(有时报的是语句开头那一行):那一行往后找不到就从头找。
        let from = line_start(sql, line).unwrap_or(0);
        return sql[from..]
            .find(snippet)
            .map(|offset| from + offset)
            .or_else(|| sql.find(snippet));
    }
    let name = quoted_after(message, "Unknown column '", '\'')
        .or_else(|| quoted_after(message, "Table '", '\''))
        .or_else(|| quoted_after(message, "FUNCTION ", ' '))?;
    find_name(sql, name, true)
}

fn sqlite_position(message: &str, sql: &str) -> Option<usize> {
    if message.contains("incomplete input") {
        return Some(end_of(sql));
    }
    let name = quoted_after(message, "near \"", '"')
        .or_else(|| message.split_once("no such table: ").map(|(_, n)| n))
        .or_else(|| message.split_once("no such column: ").map(|(_, n)| n))
        .or_else(|| message.split_once("no such function: ").map(|(_, n)| n))?;
    find_name(sql, name.trim(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_syntax_errors_point_at_the_quoted_text_on_that_line() {
        let sql = "SELECT id\nFORM orders";
        let message = "You have an error in your SQL syntax; check the manual that corresponds to your \
            MySQL server version for the right syntax to use near 'FORM orders' at line 2";
        assert_eq!(mysql_position(message, sql), Some(10));
        let end = "… for the right syntax to use near '' at line 1";
        assert_eq!(mysql_position(end, "SELECT * FROM  "), Some(13));
    }

    #[test]
    fn mysql_syntax_errors_use_the_exact_tail_when_it_is_not_cut_short() {
        let sql = "INSERT INTO t (a, b) VALUES (1, 2,)";
        let message = "… for the right syntax to use near ')' at line 1";
        assert_eq!(mysql_position(message, sql), Some(34));
    }

    #[test]
    fn names_in_comments_or_strings_or_more_than_once_are_not_marked() {
        // 这里的位置是字节:注释里的「报表」占 6 个字节。
        let commented = "-- orders 报表\nSELECT * FROM orders";
        assert_eq!(
            mysql_position("Table 'shop.orders' doesn't exist", commented),
            Some(31)
        );
        let quoted = "SELECT * FROM t WHERE x = 'status' AND status = 1";
        assert_eq!(
            mysql_position("Unknown column 'status' in 'where clause'", quoted),
            Some(39)
        );
        let twice = "SELECT nope, nope FROM t";
        assert_eq!(
            mysql_position("Unknown column 'nope' in 'field list'", twice),
            None
        );
        assert_eq!(
            sqlite_position("near \")\": syntax error", "SELECT f(a, b), g()) FROM t"),
            None
        );
        // SQLite 里 "…" 是标识符,不当字符串跳过。
        assert_eq!(
            sqlite_position("no such column: Name", "SELECT \"Name\" FROM t"),
            Some(8)
        );
    }

    #[test]
    fn mysql_names_are_found_as_whole_words() {
        let sql = "SELECT idx, o.total FROM orders o";
        assert_eq!(
            mysql_position("Unknown column 'o.total' in 'field list'", sql),
            Some(12)
        );
        assert_eq!(
            mysql_position(
                "Unknown column 'id' in 'field list'",
                "SELECT idx, id FROM t"
            ),
            Some(12)
        );
        assert_eq!(
            mysql_position(
                "Table 'shop.missing' doesn't exist",
                "SELECT * FROM missing"
            ),
            Some(14)
        );
        assert_eq!(mysql_position("Lock wait timeout exceeded", sql), None);
    }

    #[test]
    fn sqlite_errors_name_the_token() {
        assert_eq!(
            sqlite_position("near \"FORM\": syntax error", "SELECT * FORM t"),
            Some(9)
        );
        assert_eq!(
            sqlite_position("incomplete input", "SELECT * FROM  "),
            Some(13)
        );
        assert_eq!(
            sqlite_position("no such table: missing", "SELECT * FROM missing"),
            Some(14)
        );
        assert_eq!(
            sqlite_position("no such column: nope", "SELECT nope FROM t"),
            Some(7)
        );
    }

    #[test]
    fn positions_count_characters_and_lines() {
        assert_eq!(char_byte("中文 x", 3), Some(7));
        assert_eq!(char_byte("ab", 2), Some(2));
        assert_eq!(char_byte("ab", 3), None);
        assert_eq!(line_start("a\nbc\nd", 3), Some(5));
        assert_eq!(line_start("a", 2), None);
    }
}
