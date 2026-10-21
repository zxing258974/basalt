use super::*;

fn texts(record: &CsvRecord) -> Vec<(&str, bool)> {
    record
        .fields
        .iter()
        .map(|f| (f.text.as_str(), f.quoted))
        .collect()
}

#[test]
fn distinguishes_null_from_quoted_empty_string() {
    let records = parse_all("id,note\r\n1,\r\n2,\"\"\r\n", ',', '"').unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(texts(&records[1]), [("1", false), ("", false)]);
    assert_eq!(texts(&records[2]), [("2", false), ("", true)]);
    assert_eq!(records[2].number, 3);
}

#[test]
fn quoted_fields_keep_delimiters_quotes_and_newlines() {
    let records = parse_all("a,b\n\"x,y\",\"他说\"\"好\"\"\"\n\"两\r\n行\",3", ',', '"').unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(texts(&records[1]), [("x,y", true), ("他说\"好\"", true)]);
    assert_eq!(texts(&records[2]), [("两\r\n行", true), ("3", false)]);
}

#[test]
fn every_line_ending_counts_once_and_blank_lines_keep_numbers() {
    let records = parse_all("a\rb\r\n\nc\n", ',', '"').unwrap();
    let rows: Vec<(u64, bool)> = records.iter().map(|r| (r.number, r.is_blank())).collect();
    assert_eq!(rows, [(1, false), (2, false), (3, true), (4, false)]);
}

#[test]
fn last_line_without_newline_and_trailing_delimiter() {
    let records = parse_all("a,b,", ',', '"').unwrap();
    assert_eq!(
        texts(&records[0]),
        [("a", false), ("b", false), ("", false)]
    );
    let records = parse_all("\"x\"", ',', '"').unwrap();
    assert_eq!(texts(&records[0]), [("x", true)]);
    let records = parse_all("\"\"", ',', '"').unwrap();
    assert_eq!(texts(&records[0]), [("", true)]);
    assert!(parse_all("", ',', '"').unwrap().is_empty());
    assert_eq!(parse_all("\n", ',', '"').unwrap().len(), 1);
}

#[test]
fn lenient_with_stray_quotes() {
    let records = parse_all("ab\"c,\"x\"y\n", ',', '"').unwrap();
    assert_eq!(texts(&records[0]), [("ab\"c", false), ("xy", true)]);
}

#[test]
fn unclosed_quote_reports_where_it_started() {
    let error = parse_all("a\n\"open,\nmore", ',', '"').unwrap_err();
    assert!(error.contains("第 2 行"), "{error}");
}

#[test]
fn feeding_in_pieces_gives_the_same_records() {
    let text = "名字,备注\r\n\"张\"\"三\",\"多\r\n行\"\r\n李四,\r\n";
    let whole = parse_all(text, ',', '"').unwrap();
    let mut parser = CsvParser::new(',', '"');
    let mut pieces = Vec::new();
    for c in text.chars() {
        parser.feed(&c.to_string(), &mut pieces);
    }
    parser.finish(&mut pieces).unwrap();
    assert_eq!(pieces, whole);
}

#[test]
fn custom_delimiter_and_quote() {
    let records = parse_all("'a;b';'it''s'\n", ';', '\'').unwrap();
    assert_eq!(texts(&records[0]), [("a;b", true), ("it's", true)]);
}

#[test]
fn detects_the_delimiter_from_a_sample() {
    assert_eq!(detect_delimiter("a,b,c\n1,2,3\n4,5,6\n", '"'), ',');
    assert_eq!(detect_delimiter("a\tb\n1\t2\n", '"'), '\t');
    assert_eq!(detect_delimiter("a;b;c\n\"1,5\";2;3\n", '"'), ';');
    assert_eq!(detect_delimiter("a|b\n1|2\n3|4\nincomplete|", '"'), '|');
    // 只有一列时退回逗号。
    assert_eq!(detect_delimiter("name\n张三\n", '"'), ',');
    assert_eq!(detect_delimiter("", '"'), ',');
}
