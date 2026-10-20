//! CSV 解析器:逐字符喂入、逐条吐出记录,可以一块一块地读大文件。
//!
//! 自己写而不用 csv crate:导入要分清「没加引号的空字段」(NULL)和「一对引号」(空字符串),
//! csv crate 不告诉调用方一个字段有没有引号。规则按 RFC 4180,对常见的不规范写法宽容:
//! 不加引号的字段里出现引号照原样保留,闭合引号后面跟着别的字符就接在后面。
//! 换行认 CRLF、LF 和单独的 CR。

/// 一个字段:原文与有没有用引号括起来。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvField {
    pub text: String,
    pub quoted: bool,
}

impl CsvField {
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            quoted: false,
        }
    }
}

/// 一条记录与它是文件里的第几条(从 1 开始,空行也算,和电子表格里的行号一致)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvRecord {
    pub number: u64,
    pub fields: Vec<CsvField>,
}

impl CsvRecord {
    /// 空行:只有一个没加引号的空字段。
    pub fn is_blank(&self) -> bool {
        matches!(self.fields.as_slice(), [field] if field.text.is_empty() && !field.quoted)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    FieldStart,
    Unquoted,
    Quoted,
    /// 引号括起来的字段里遇到了一个引号:可能是转义的两个引号,也可能是结束。
    QuoteInQuoted,
    /// 刚结束一条以 CR 结尾的记录,后面紧跟的 LF 不再算一行。
    AfterCr,
}

pub struct CsvParser {
    delimiter: char,
    quote: char,
    state: State,
    field: String,
    quoted: bool,
    fields: Vec<CsvField>,
    /// 当前记录已经读到内容(哪怕只是一个分隔符)。
    open: bool,
    number: u64,
}

impl CsvParser {
    pub fn new(delimiter: char, quote: char) -> Self {
        Self {
            delimiter,
            quote,
            state: State::FieldStart,
            field: String::new(),
            quoted: false,
            fields: Vec::new(),
            open: false,
            number: 0,
        }
    }

    fn end_field(&mut self) {
        self.fields.push(CsvField {
            text: std::mem::take(&mut self.field),
            quoted: self.quoted,
        });
        self.quoted = false;
    }

    fn end_record(&mut self, out: &mut Vec<CsvRecord>) {
        self.end_field();
        self.number += 1;
        out.push(CsvRecord {
            number: self.number,
            fields: std::mem::take(&mut self.fields),
        });
        self.open = false;
    }

    /// 喂一段文本,读完整的记录追加到 `out`。
    pub fn feed(&mut self, text: &str, out: &mut Vec<CsvRecord>) {
        for c in text.chars() {
            self.push(c, out);
        }
    }

    fn push(&mut self, c: char, out: &mut Vec<CsvRecord>) {
        if self.state == State::AfterCr {
            self.state = State::FieldStart;
            if c == '\n' {
                return;
            }
        }
        if self.state != State::Quoted && self.state != State::QuoteInQuoted {
            self.open = self.open || !matches!(c, '\r' | '\n');
        }
        match (self.state, c) {
            (State::Quoted, c) if c == self.quote => self.state = State::QuoteInQuoted,
            (State::Quoted, c) => self.field.push(c),
            (State::QuoteInQuoted, c) if c == self.quote => {
                self.field.push(c);
                self.state = State::Quoted;
            }
            (State::FieldStart, c) if c == self.quote => {
                self.quoted = true;
                self.open = true;
                self.state = State::Quoted;
            }
            (_, c) if c == self.delimiter => {
                self.end_field();
                self.state = State::FieldStart;
            }
            (_, '\r') => {
                self.end_record(out);
                self.state = State::AfterCr;
            }
            (_, '\n') => {
                self.end_record(out);
                self.state = State::FieldStart;
            }
            (_, c) => {
                self.field.push(c);
                self.state = State::Unquoted;
            }
        }
    }

    /// 文件读完了:最后一行没有换行符也算一条。引号没闭合就是错。
    pub fn finish(&mut self, out: &mut Vec<CsvRecord>) -> Result<(), String> {
        if self.state == State::Quoted {
            return Err(format!(
                "第 {} 行开始的引号一直没有闭合，文件可能不完整，或者引号字符选错了。",
                self.number + 1
            ));
        }
        if self.open || !self.fields.is_empty() || self.quoted {
            self.end_record(out);
        }
        Ok(())
    }
}

/// 一次性解析一段完整的文本(探测分隔符、测试用)。
pub fn parse_all(text: &str, delimiter: char, quote: char) -> Result<Vec<CsvRecord>, String> {
    let mut parser = CsvParser::new(delimiter, quote);
    let mut out = Vec::new();
    parser.feed(text, &mut out);
    parser.finish(&mut out)?;
    Ok(out)
}

pub const DELIMITER_CANDIDATES: [char; 4] = [',', '\t', ';', '|'];

/// 看开头一段猜分隔符:每行字段数都一样且大于 1 的候选里取字段最多的;都不一致就取平均字段最多的。
/// 样本最后一行可能不完整,不算。
pub fn detect_delimiter(sample: &str, quote: char) -> char {
    let complete = match sample.rfind('\n') {
        Some(end) => &sample[..end],
        None => sample,
    };
    let mut best = (false, 0.0_f64, ',');
    for candidate in DELIMITER_CANDIDATES {
        let mut parser = CsvParser::new(candidate, quote);
        let mut records = Vec::new();
        parser.feed(complete, &mut records);
        // 引号没闭合说明样本切在了多行字段中间,照样用已经读到的记录。
        let _ = parser.finish(&mut records);
        let counts: Vec<usize> = records
            .iter()
            .filter(|r| !r.is_blank())
            .take(50)
            .map(|r| r.fields.len())
            .collect();
        let Some(&first) = counts.first() else {
            continue;
        };
        let consistent = first > 1 && counts.iter().all(|&n| n == first);
        let average = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        if (consistent, average) > (best.0, best.1) {
            best = (consistent, average, candidate);
        }
    }
    best.2
}

#[cfg(test)]
#[path = "csv_read_tests.rs"]
mod tests;
