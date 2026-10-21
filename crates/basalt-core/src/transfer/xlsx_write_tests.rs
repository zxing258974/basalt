use chrono::NaiveDate;

use super::*;
use crate::transfer::source::{SourceRecord, SourceValue};
use crate::transfer::xlsx_read::{read_sheet, sheet_names};

fn round_trip(writer: XlsxWriter, columns: &[&str], rows: &[Vec<CellValue>]) -> Vec<SourceRecord> {
    let path = writer.path.clone();
    let mut writer = writer;
    let columns: Vec<ExportColumn> = columns.iter().map(|n| ExportColumn::new(n, "")).collect();
    writer.begin(&columns).unwrap();
    for row in rows {
        writer.write_row(row).unwrap();
    }
    writer.finish().unwrap();
    let mut records = Vec::new();
    for sheet in sheet_names(path.to_str().unwrap()).unwrap() {
        read_sheet(path.to_str().unwrap(), &sheet, &mut |record| {
            records.push(record);
            Ok(true)
        })
        .unwrap();
    }
    records
}

#[test]
fn values_keep_their_meaning_after_a_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let writer = XlsxWriter::new(dir.path().join("a.xlsx"), "用户", BinaryEncoding::Hex);
    let records = round_trip(
        writer,
        &[
            "id",
            "big",
            "price",
            "long",
            "名字",
            "空",
            "无",
            "生日",
            "时间",
            "二进制",
            "真",
        ],
        &[vec![
            CellValue::Int(42),
            CellValue::Int(9_007_199_254_740_993),
            CellValue::Decimal("12.50".into()),
            CellValue::Decimal("12345678901234567.89".into()),
            CellValue::Text("张三".into()),
            CellValue::Text(String::new()),
            CellValue::Null,
            CellValue::Date("2026-10-20".into()),
            CellValue::DateTime("2026-10-20 08:05:09.12".into()),
            CellValue::Bytes(vec![0xab, 0x01]),
            CellValue::Bool(true),
        ]],
    );
    assert_eq!(records.len(), 2, "列名行 + 一行数据");
    assert_eq!(records[0].row, 1);
    assert_eq!(records[0].values[4], SourceValue::Text("名字".into()));
    let values = &records[1].values;
    assert_eq!(records[1].row, 2);
    assert_eq!(values[0], SourceValue::Number(42.0));
    assert_eq!(values[1], SourceValue::Text("9007199254740993".into()));
    assert_eq!(values[2], SourceValue::Number(12.5));
    assert_eq!(values[3], SourceValue::Text("12345678901234567.89".into()));
    assert_eq!(values[4], SourceValue::Text("张三".into()));
    assert_eq!(
        values[5],
        SourceValue::Text(String::new()),
        "空字符串不是 NULL"
    );
    assert_eq!(values[6], SourceValue::Null);
    let birthday = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    assert_eq!(
        values[7],
        SourceValue::DateTime(birthday.and_hms_opt(0, 0, 0).unwrap())
    );
    assert_eq!(
        values[8],
        SourceValue::DateTime(birthday.and_hms_milli_opt(8, 5, 9, 120).unwrap())
    );
    assert_eq!(values[9], SourceValue::Text("0xab01".into()));
    assert_eq!(values[10], SourceValue::Bool(true));
}

#[test]
fn values_excel_cannot_hold_exactly_become_text() {
    let base64 = BinaryEncoding::Base64;
    let as_text = |value: CellValue| match cell_for(&value, base64).unwrap() {
        XlsxCell::Text(text) => text,
        other => panic!("{value:?} 应该写成文本，实际是 {other:?}"),
    };
    assert_eq!(
        as_text(CellValue::DateTime("2026-10-20 08:05:09.123456".into())),
        "2026-10-20 08:05:09.123456"
    );
    assert_eq!(
        as_text(CellValue::DateTime("2026-10-20 08:05:09+08:00".into())),
        "2026-10-20 08:05:09+08:00"
    );
    assert_eq!(as_text(CellValue::Date("0000-00-00".into())), "0000-00-00");
    assert_eq!(as_text(CellValue::Date("1899-01-01".into())), "1899-01-01");
    assert_eq!(as_text(CellValue::Time("-838:59:59".into())), "-838:59:59");
    assert_eq!(as_text(CellValue::Float(f64::INFINITY)), "Infinity");
    assert_eq!(as_text(CellValue::UInt(u64::MAX)), "18446744073709551615");
    assert_eq!(as_text(CellValue::Bytes(vec![1])), "AQ==");
    assert_eq!(
        cell_for(&CellValue::Int(999_999_999_999_999), base64).unwrap(),
        XlsxCell::Number(999_999_999_999_999.0)
    );
    assert_eq!(
        cell_for(&CellValue::Decimal("-0.000123".into()), base64).unwrap(),
        XlsxCell::Number(-0.000123)
    );
    let long = CellValue::Text("字".repeat(32_768));
    assert!(cell_for(&long, base64).unwrap_err().contains("32767"));
    assert!(cell_for(&CellValue::unsupported("GEOMETRY"), base64).is_err());
}

#[test]
fn rows_past_the_sheet_limit_continue_on_a_new_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let writer = XlsxWriter::new(dir.path().join("b.xlsx"), "订单", BinaryEncoding::Hex)
        .with_rows_per_sheet(2);
    let rows: Vec<Vec<CellValue>> = (1..=5).map(|n| vec![CellValue::Int(n)]).collect();
    let records = round_trip(writer, &["id"], &rows);
    // 三个工作表,各自一行列名。
    let headers = records
        .iter()
        .filter(|r| r.values[0] == SourceValue::Text("id".into()))
        .count();
    assert_eq!(headers, 3);
    assert_eq!(records.len(), 8);
    let names = sheet_names(dir.path().join("b.xlsx").to_str().unwrap()).unwrap();
    assert_eq!(names, ["订单", "订单 (2)", "订单 (3)"]);
}

#[test]
fn sheet_names_are_cleaned_and_shortened() {
    assert_eq!(sheet_name("a/b:c", 1), "abc");
    assert_eq!(sheet_name("", 1), "Sheet");
    assert_eq!(sheet_name("'quoted'", 1), "quoted");
    let long = "很".repeat(40);
    assert_eq!(sheet_name(&long, 1).chars().count(), 31);
    assert_eq!(sheet_name(&long, 12).chars().count(), 31);
    assert!(sheet_name(&long, 12).ends_with(" (12)"));
}

#[test]
fn significant_digits_ignore_leading_and_trailing_zeros() {
    assert_eq!(significant_digits("0.00120"), Some(2));
    assert_eq!(significant_digits("-1000"), Some(1));
    assert_eq!(significant_digits("123456789012345.6"), Some(16));
    assert_eq!(significant_digits("1e5"), None);
    assert_eq!(significant_digits("."), None);
}
