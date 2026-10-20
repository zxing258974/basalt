//! 从 SQLite 的 CREATE TABLE 原文里取出 PRAGMA 给不了的东西:字段的排序规则、生成列表达式、
//! AUTOINCREMENT、检查约束、外键名、WITHOUT ROWID / STRICT。重建表时要靠它们原样保留。
//!
//! 只做到够用的词法切分:字符串、带引号的标识符、注释、括号嵌套都认得,不做完整语法分析。

use super::Generated;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    /// 不带引号的词(关键字或标识符)。
    Word,
    /// 带引号的标识符:"x"、`x`、[x]。
    Quoted,
    Str,
    Other,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    /// 标识符的值(去掉引号);其余是原文。
    text: String,
    start: usize,
    end: usize,
}

impl Token {
    fn is(&self, keyword: &str) -> bool {
        self.kind == Kind::Word && self.text.eq_ignore_ascii_case(keyword)
    }

    fn is_punct(&self, c: char) -> bool {
        self.kind == Kind::Other && self.text.len() == 1 && self.text.starts_with(c)
    }

    fn is_name(&self) -> bool {
        matches!(self.kind, Kind::Word | Kind::Quoted | Kind::Str)
    }
}

fn tokenize(sql: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = sql.char_indices().collect();
    let byte_at = |i: usize| chars.get(i).map_or(sql.len(), |(b, _)| *b);
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        let next = chars.get(i + 1).map(|(_, c)| *c);
        let start = i;
        let (kind, end) = match c {
            _ if c.is_whitespace() => {
                i += 1;
                continue;
            }
            '-' if next == Some('-') => {
                i = (i..chars.len())
                    .find(|&j| chars[j].1 == '\n')
                    .unwrap_or(chars.len());
                continue;
            }
            '/' if next == Some('*') => {
                let close = (i + 2..chars.len())
                    .find(|&j| chars[j].1 == '*' && chars.get(j + 1).map(|c| c.1) == Some('/'));
                i = close.map_or(chars.len(), |j| j + 2);
                continue;
            }
            '\'' => (Kind::Str, quoted_end(&chars, i, '\'')),
            '"' => (Kind::Quoted, quoted_end(&chars, i, '"')),
            '`' => (Kind::Quoted, quoted_end(&chars, i, '`')),
            '[' => (
                Kind::Quoted,
                (i..chars.len())
                    .find(|&j| chars[j].1 == ']')
                    .map_or(chars.len(), |j| j + 1),
            ),
            _ if c.is_alphanumeric() || c == '_' || c == '$' => (
                Kind::Word,
                (i..chars.len())
                    .find(|&j| !(chars[j].1.is_alphanumeric() || matches!(chars[j].1, '_' | '$')))
                    .unwrap_or(chars.len()),
            ),
            _ => (Kind::Other, i + 1),
        };
        let raw = &sql[byte_at(start)..byte_at(end)];
        let text = match kind {
            Kind::Quoted => unquote(raw),
            _ => raw.to_owned(),
        };
        tokens.push(Token {
            kind,
            text,
            start: byte_at(start),
            end: byte_at(end),
        });
        i = end;
    }
    tokens
}

/// 引号内的同种引号成对表示一个。返回结束引号之后的位置。
fn quoted_end(chars: &[(usize, char)], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i].1 == quote {
            if chars.get(i + 1).map(|c| c.1) == Some(quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

fn unquote(raw: &str) -> String {
    let inner = &raw[1..raw.len().saturating_sub(1).max(1)];
    match raw.chars().next() {
        Some('"') => inner.replace("\"\"", "\""),
        Some('`') => inner.replace("``", "`"),
        _ => inner.to_owned(),
    }
}

/// 一段 SQL 里出现的所有名字(小写),用来粗查谁引用了某个字段或表。
pub(crate) fn identifiers(sql: &str) -> Vec<String> {
    tokenize(sql)
        .into_iter()
        .filter(|t| matches!(t.kind, Kind::Word | Kind::Quoted))
        .map(|t| t.text.to_lowercase())
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ParsedColumn {
    pub name: String,
    pub collation: Option<String>,
    pub generated: Option<Generated>,
    pub autoincrement: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ParsedTable {
    pub columns: Vec<ParsedColumn>,
    /// 表级与字段级的检查约束:(约束名, 表达式)。
    pub checks: Vec<(Option<String>, String)>,
    /// 有名字的外键:(本表字段名小写, 约束名)。
    pub foreign_key_names: Vec<(Vec<String>, String)>,
    pub without_rowid: bool,
    pub strict: bool,
}

/// 解析不了(不是 CREATE TABLE … ( … ) 的形状)时返回 `None`。
pub(crate) fn parse_create_table(sql: &str) -> Option<ParsedTable> {
    let tokens = tokenize(sql);
    let open = tokens.iter().position(|t| t.is_punct('('))?;
    let (segments, close) = split_top_level(&tokens, open)?;
    let mut table = ParsedTable::default();
    for segment in segments {
        parse_segment(sql, segment, &mut table);
    }
    for token in &tokens[close + 1..] {
        table.without_rowid |= token.is("ROWID");
        table.strict |= token.is("STRICT");
    }
    Some(table)
}

/// 从 `open` 处的左括号起,按顶层逗号切开,返回各段和右括号的位置。
fn split_top_level(tokens: &[Token], open: usize) -> Option<(Vec<&[Token]>, usize)> {
    let mut depth = 0;
    let mut segments = Vec::new();
    let mut start = open + 1;
    for (i, token) in tokens.iter().enumerate().skip(open) {
        if token.is_punct('(') {
            depth += 1;
        } else if token.is_punct(')') {
            depth -= 1;
            if depth == 0 {
                segments.push(&tokens[start..i]);
                return Some((segments, i));
            }
        } else if token.is_punct(',') && depth == 1 {
            segments.push(&tokens[start..i]);
            start = i + 1;
        }
    }
    None
}

/// `tokens[open]` 是左括号:返回括号里的原文和右括号之后的位置。
fn parenthesized(sql: &str, tokens: &[Token], open: usize) -> Option<(String, usize)> {
    if !tokens.get(open)?.is_punct('(') {
        return None;
    }
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate().skip(open) {
        if token.is_punct('(') {
            depth += 1;
        } else if token.is_punct(')') {
            depth -= 1;
            if depth == 0 {
                let inner = &sql[tokens[open].end..token.start];
                return Some((inner.trim().to_owned(), i + 1));
            }
        }
    }
    None
}

fn parse_segment(sql: &str, segment: &[Token], table: &mut ParsedTable) {
    let Some(first) = segment.first() else { return };
    let (name, rest) = if first.is("CONSTRAINT") {
        (
            segment.get(1).map(|t| t.text.clone()),
            segment.get(2..).unwrap_or_default(),
        )
    } else {
        (None, segment)
    };
    let Some(head) = rest.first() else { return };
    if head.is("CHECK") {
        if let Some((expression, _)) = parenthesized(sql, rest, 1) {
            table.checks.push((name, expression));
        }
    } else if head.is("FOREIGN") {
        if let (Some(name), Some(open)) = (name, rest.iter().position(|t| t.is_punct('('))) {
            let columns = column_names(rest, open);
            table.foreign_key_names.push((columns, name));
        }
    } else if !(head.is("PRIMARY") || head.is("UNIQUE")) && head.is_name() {
        let column = parse_column(sql, segment, table);
        table.columns.push(column);
    }
}

fn column_names(tokens: &[Token], open: usize) -> Vec<String> {
    tokens[open + 1..]
        .iter()
        .take_while(|t| !t.is_punct(')'))
        .filter(|t| t.is_name())
        .map(|t| t.text.to_lowercase())
        .collect()
}

fn parse_column(sql: &str, tokens: &[Token], table: &mut ParsedTable) -> ParsedColumn {
    let mut column = ParsedColumn {
        name: tokens[0].text.clone(),
        ..ParsedColumn::default()
    };
    let mut constraint = None;
    let mut i = 1;
    while i < tokens.len() {
        let token = &tokens[i];
        if token.is_punct('(') {
            // 类型参数、默认值表达式之类,整体跳过。
            i = parenthesized(sql, tokens, i).map_or(tokens.len(), |(_, end)| end);
            continue;
        }
        if token.is("CONSTRAINT") {
            constraint = tokens.get(i + 1).map(|t| t.text.clone());
            i += 2;
            continue;
        }
        if token.is("COLLATE") {
            column.collation = tokens.get(i + 1).map(|t| t.text.clone());
        } else if token.is("AUTOINCREMENT") {
            column.autoincrement = true;
        } else if token.is("CHECK") {
            if let Some((expression, end)) = parenthesized(sql, tokens, i + 1) {
                table.checks.push((constraint.take(), expression));
                i = end;
                continue;
            }
        } else if token.is("AS")
            && let Some((expression, end)) = parenthesized(sql, tokens, i + 1)
        {
            let stored = tokens.get(end).is_some_and(|t| t.is("STORED"));
            column.generated = Some(Generated { expression, stored });
            i = end;
            continue;
        }
        i += 1;
    }
    column
}

#[cfg(test)]
#[path = "sqlite_parse_tests.rs"]
mod tests;
