use super::*;

fn spec(path: &std::path::Path, has_header: bool) -> SourceSpec {
    SourceSpec {
        path: path.to_string_lossy().into_owned(),
        format: None,
        has_header,
    }
}

fn collect(spec: &SourceSpec) -> (Vec<String>, Vec<SourceRecord>) {
    let (format, _) = resolve_format(spec).unwrap();
    let resolved = SourceSpec {
        format: Some(format),
        ..spec.clone()
    };
    let mut headers = Vec::new();
    let mut records = Vec::new();
    read_rows(
        &resolved,
        |names| headers = names,
        |record| {
            records.push(record);
            Ok(true)
        },
    )
    .unwrap();
    (headers, records)
}

#[test]
fn csv_rows_keep_null_empty_and_file_row_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.csv");
    std::fs::write(
        &path,
        "id,名字,备注\r\n1,张三,\r\n\r\n2,\"\",\"多\n行\"\r\n",
    )
    .unwrap();
    let (headers, records) = collect(&spec(&path, true));
    assert_eq!(headers, ["id", "名字", "备注"]);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].row, 2);
    assert_eq!(records[0].values[2], SourceValue::Null);
    assert_eq!(records[1].row, 4, "空行也算行号");
    assert_eq!(records[1].values[1], SourceValue::Text(String::new()));
    assert_eq!(records[1].values[2], SourceValue::Text("多\n行".into()));
}

#[test]
fn gbk_files_and_semicolons_are_detected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gbk.csv");
    let (bytes, _, _) = encoding_rs::GBK.encode("编号;名称\n1;中文\n");
    std::fs::write(&path, &bytes).unwrap();
    let preview = inspect(&spec(&path, true)).unwrap();
    assert_eq!(
        preview.format,
        SourceFormat::Csv {
            encoding: Some(TextEncoding::Gbk),
            delimiter: Some(';'),
            quote: '"'
        }
    );
    assert_eq!(preview.headers, ["编号", "名称"]);
    assert_eq!(preview.rows, [vec![Some("1".into()), Some("中文".into())]]);
    assert_eq!(preview.row_numbers, [2]);
}

#[test]
fn without_a_header_row_columns_are_numbered_and_preview_stops_early() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.tsv");
    let text: String = (1..=50).map(|n| format!("{n}\tx{n}\n")).collect();
    std::fs::write(&path, text).unwrap();
    let preview = inspect(&spec(&path, false)).unwrap();
    assert_eq!(preview.headers, ["第 1 列", "第 2 列"]);
    assert_eq!(preview.rows.len(), PREVIEW_ROWS);
    assert_eq!(preview.row_numbers[0], 1);
}

#[test]
fn wrong_encoding_and_unclosed_quotes_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.csv");
    let (bytes, _, _) = encoding_rs::GBK.encode("名\n中文\n");
    std::fs::write(&path, &bytes).unwrap();
    let forced = SourceSpec {
        format: Some(SourceFormat::Csv {
            encoding: Some(TextEncoding::Utf8),
            delimiter: Some(','),
            quote: '"',
        }),
        ..spec(&path, true)
    };
    let error = read_rows(&forced, |_| {}, |_| Ok(true)).unwrap_err();
    assert_eq!(error.code(), "invalid_data");

    std::fs::write(&path, "a\n\"open\n").unwrap();
    let error = inspect(&spec(&path, true)).unwrap_err();
    assert!(error.to_string().contains("引号"), "{error}");
}

#[test]
fn format_comes_from_the_extension() {
    assert!(matches!(
        format_for_path("/a/b.XLSX"),
        Ok(SourceFormat::Excel { sheet: None })
    ));
    assert!(matches!(
        format_for_path("x.tsv"),
        Ok(SourceFormat::Csv {
            delimiter: Some('\t'),
            ..
        })
    ));
    assert_eq!(format_for_path("x.pdf").unwrap_err().code(), "unsupported");
    let unresolved = SourceSpec {
        path: "x.csv".into(),
        format: Some(SourceFormat::Csv {
            encoding: None,
            delimiter: None,
            quote: '"',
        }),
        has_header: true,
    };
    assert!(read_rows(&unresolved, |_| {}, |_| Ok(true)).is_err());
}

#[test]
fn excel_preview_lists_sheets_and_rejects_unknown_ones() {
    use crate::transfer::writer::RowWriter;
    use crate::transfer::xlsx_write::XlsxWriter;
    use crate::transfer::{BinaryEncoding, ExportColumn};
    use crate::value::CellValue;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.xlsx");
    let mut writer = XlsxWriter::new(path.clone(), "数据", BinaryEncoding::Hex);
    writer
        .begin(&[ExportColumn::new("编号", ""), ExportColumn::new("", "")])
        .unwrap();
    writer
        .write_row(&[CellValue::Int(7), CellValue::Date("2026-10-20".into())])
        .unwrap();
    writer.finish().unwrap();

    let preview = inspect(&spec(&path, true)).unwrap();
    assert_eq!(preview.sheets, ["数据"]);
    assert_eq!(
        preview.format,
        SourceFormat::Excel {
            sheet: Some("数据".into())
        }
    );
    assert_eq!(preview.headers, ["编号", "第 2 列"]);
    assert_eq!(
        preview.rows,
        [vec![Some("7".into()), Some("2026-10-20".into())]]
    );

    let missing = SourceSpec {
        format: Some(SourceFormat::Excel {
            sheet: Some("没有".into()),
        }),
        ..spec(&path, true)
    };
    assert_eq!(inspect(&missing).unwrap_err().code(), "not_found");
}

#[test]
fn display_of_excel_values() {
    use chrono::NaiveDate;
    let date = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap();
    assert_eq!(
        SourceValue::DateTime(date.and_hms_opt(13, 5, 0).unwrap())
            .display()
            .unwrap(),
        "13:05:00"
    );
    assert_eq!(SourceValue::Number(1.5).display().unwrap(), "1.5");
    assert_eq!(SourceValue::Bool(false).display().unwrap(), "FALSE");
    assert_eq!(SourceValue::Error("#N/A".into()).display().unwrap(), "#N/A");
    assert_eq!(SourceValue::Null.display(), None);
}
