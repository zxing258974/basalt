use chrono::NaiveDate;

use super::*;
use crate::transfer::target::classify;

fn col(data_type: &str) -> ColumnInfo {
    ColumnInfo::new("c", data_type)
}

fn run(dialect: Dialect, data_type: &str, value: SourceValue) -> Result<BindValue, String> {
    let column = col(data_type);
    let kind = classify(dialect, &column);
    convert(dialect, &column, &kind, &value, BinaryEncoding::Hex)
}

fn text(value: &str) -> SourceValue {
    SourceValue::Text(value.into())
}

fn ok(dialect: Dialect, data_type: &str, value: SourceValue) -> BindValue {
    run(dialect, data_type, value.clone())
        .unwrap_or_else(|e| panic!("{data_type} ← {value:?}：{e}"))
}

fn err(dialect: Dialect, data_type: &str, value: SourceValue) -> String {
    run(dialect, data_type, value.clone()).expect_err(&format!("{data_type} ← {value:?} 应该失败"))
}

#[test]
fn null_needs_a_nullable_column_and_excel_errors_fail() {
    let column = ColumnInfo {
        nullable: false,
        ..col("int")
    };
    let kind = classify(Dialect::Mysql, &column);
    let error = convert(
        Dialect::Mysql,
        &column,
        &kind,
        &SourceValue::Null,
        BinaryEncoding::Hex,
    )
    .unwrap_err();
    assert!(error.contains("不能为空"));
    assert_eq!(
        ok(Dialect::Mysql, "int", SourceValue::Null),
        BindValue::Null
    );
    assert!(
        err(
            Dialect::Mysql,
            "varchar(9)",
            SourceValue::Error("#N/A".into())
        )
        .contains("#N/A")
    );
}

#[test]
fn integers_check_syntax_range_and_excel_precision() {
    assert_eq!(ok(Dialect::Mysql, "int", text(" 42 ")), BindValue::Int(42));
    assert_eq!(
        ok(
            Dialect::Mysql,
            "bigint unsigned",
            text("18446744073709551615")
        ),
        BindValue::UInt(u64::MAX)
    );
    assert_eq!(
        ok(Dialect::Mysql, "tinyint(1)", SourceValue::Bool(true)),
        BindValue::Int(1)
    );
    assert_eq!(
        ok(Dialect::Sqlite, "INTEGER", SourceValue::Number(7.0)),
        BindValue::Int(7)
    );
    assert_eq!(
        ok(Dialect::Postgres, "bigint", SourceValue::Integer(-3)),
        BindValue::Text("-3".into())
    );
    assert!(err(Dialect::Mysql, "tinyint", text("200")).contains("-128 到 127"));
    assert!(err(Dialect::Mysql, "int unsigned", text("-1")).contains("范围"));
    assert!(err(Dialect::Mysql, "int", text("4x2")).contains("整数"));
    assert!(err(Dialect::Mysql, "int", text("5.0")).contains("整数"));
    assert!(err(Dialect::Mysql, "int", text("")).contains("整数"));
    assert!(err(Dialect::Sqlite, "INTEGER", SourceValue::Number(1.5)).contains("不是整数"));
    assert!(err(Dialect::Mysql, "bigint", SourceValue::Number(1e17)).contains("15 位"));
    let date = NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    assert!(err(Dialect::Mysql, "int", SourceValue::DateTime(date)).contains("日期"));
    assert_eq!(
        ok(Dialect::Mysql, "year", text("2026")),
        BindValue::Int(2026)
    );
    assert_eq!(ok(Dialect::Mysql, "year", text("0")), BindValue::Int(0));
    assert!(err(Dialect::Mysql, "year", text("1800")).contains("1901"));
}

#[test]
fn floats_and_decimals_keep_their_digits() {
    assert_eq!(
        ok(Dialect::Mysql, "double", text("0.1")),
        BindValue::Float(0.1)
    );
    assert_eq!(
        ok(Dialect::Postgres, "real", text("3.14159265358979")),
        BindValue::Text("3.14159265358979".into())
    );
    assert_eq!(
        ok(Dialect::Postgres, "double precision", text("NaN")),
        BindValue::Text("NaN".into())
    );
    assert!(err(Dialect::Mysql, "double", text("NaN")).contains("NaN"));
    assert!(err(Dialect::Mysql, "double", text("1e400")).contains("范围"));
    assert!(err(Dialect::Mysql, "double", text("abc")).contains("数字"));
    assert_eq!(
        ok(Dialect::Sqlite, "REAL", SourceValue::Integer(3)),
        BindValue::Float(3.0)
    );

    assert_eq!(
        ok(
            Dialect::Mysql,
            "decimal(40,10)",
            text("123456789012345678901234567890.0123456789")
        ),
        BindValue::Text("123456789012345678901234567890.0123456789".into())
    );
    assert_eq!(
        ok(Dialect::Mysql, "decimal(10,2)", SourceValue::Number(12.5)),
        BindValue::Text("12.5".into())
    );
    assert_eq!(
        ok(Dialect::Mysql, "decimal(10,2)", text("12.500")),
        BindValue::Text("12.500".into())
    );
    assert!(err(Dialect::Mysql, "decimal(10,2)", text("1.234")).contains("2 位"));
    assert!(err(Dialect::Mysql, "decimal(5,2)", text("1234")).contains("3 位"));
    assert!(err(Dialect::Mysql, "decimal(10,2)", text("12,5")).contains("数字"));
    assert_eq!(
        ok(Dialect::Postgres, "numeric", text("-Infinity")),
        BindValue::Text("-Infinity".into())
    );
    assert!(err(Dialect::Mysql, "decimal(10,2)", text("NaN")).contains("数字"));
    assert_eq!(
        ok(Dialect::Sqlite, "DECIMAL(10,2)", text("1.23456")),
        BindValue::Text("1.23456".into())
    );
}

#[test]
fn decimal_digits_handle_exponents_and_zeros() {
    assert_eq!(decimal_digits("00123.4500"), Some((3, 2)));
    assert_eq!(decimal_digits("1.5e3"), Some((4, 0)));
    assert_eq!(decimal_digits("1e-3"), Some((0, 3)));
    assert_eq!(decimal_digits("-0.000"), Some((0, 0)));
    assert_eq!(decimal_digits(".5"), Some((0, 1)));
    assert_eq!(decimal_digits("1e"), None);
    assert_eq!(decimal_digits("1.2.3"), None);
}

#[test]
fn bits_booleans_and_binary() {
    assert_eq!(
        ok(Dialect::Mysql, "bit(8)", text("255")),
        BindValue::Int(255)
    );
    assert_eq!(
        ok(Dialect::Mysql, "bit(4)", text("b'1010'")),
        BindValue::Int(10)
    );
    assert!(err(Dialect::Mysql, "bit(4)", text("16")).contains("BIT(4)"));
    assert_eq!(
        ok(Dialect::Postgres, "boolean", text("T")),
        BindValue::Text("true".into())
    );
    assert_eq!(
        ok(Dialect::Postgres, "boolean", SourceValue::Number(0.0)),
        BindValue::Text("false".into())
    );
    assert_eq!(
        ok(Dialect::Sqlite, "BOOLEAN", text("yes")),
        BindValue::Int(1)
    );
    assert!(err(Dialect::Postgres, "boolean", text("maybe")).contains("布尔"));

    assert_eq!(
        ok(Dialect::Mysql, "blob", text("0xAB01")),
        BindValue::Bytes(vec![0xab, 0x01])
    );
    assert_eq!(
        ok(Dialect::Mysql, "blob", text("plain")),
        BindValue::Bytes(b"plain".to_vec())
    );
    assert_eq!(
        ok(Dialect::Postgres, "bytea", text("0x00ff")),
        BindValue::Text("\\x00ff".into())
    );
    assert!(err(Dialect::Mysql, "blob", text("0xZZ")).contains("十六进制"));
    let column = col("blob");
    let kind = classify(Dialect::Mysql, &column);
    let base64 = convert(
        Dialect::Mysql,
        &column,
        &kind,
        &text("AP8="),
        BinaryEncoding::Base64,
    );
    assert_eq!(base64.unwrap(), BindValue::Bytes(vec![0x00, 0xff]));
    assert!(
        convert(
            Dialect::Mysql,
            &column,
            &kind,
            &text("@@"),
            BinaryEncoding::Base64
        )
        .is_err()
    );
}

#[test]
fn dates_and_times_are_validated_per_dialect() {
    let day = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    assert_eq!(
        ok(Dialect::Mysql, "date", text("2026/10/20")),
        BindValue::Text("2026-10-20".into())
    );
    assert_eq!(
        ok(Dialect::Mysql, "date", text("2026-10-20 00:00:00")),
        BindValue::Text("2026-10-20".into())
    );
    assert_eq!(
        ok(
            Dialect::Mysql,
            "date",
            SourceValue::DateTime(day.and_hms_opt(0, 0, 0).unwrap())
        ),
        BindValue::Text("2026-10-20".into())
    );
    assert_eq!(
        ok(Dialect::Mysql, "date", text("0000-00-00")),
        BindValue::Text("0000-00-00".into())
    );
    assert!(err(Dialect::Postgres, "date", text("0000-00-00")).contains("日期"));
    assert!(err(Dialect::Mysql, "date", text("2026-02-30")).contains("日期"));
    assert!(err(Dialect::Mysql, "date", text("2026-10-20 08:00:00")).contains("时间部分"));
    assert!(
        err(
            Dialect::Mysql,
            "date",
            SourceValue::DateTime(day.and_hms_opt(8, 0, 0).unwrap())
        )
        .contains("时间部分")
    );
    assert_eq!(
        ok(Dialect::Postgres, "date", text("infinity")),
        BindValue::Text("infinity".into())
    );
    assert_eq!(
        ok(Dialect::Postgres, "date", text("0044-03-15 BC")),
        BindValue::Text("0044-03-15 BC".into())
    );

    assert_eq!(
        ok(
            Dialect::Mysql,
            "datetime(6)",
            text("2026-10-20T08:05:09.123456")
        ),
        BindValue::Text("2026-10-20 08:05:09.123456".into())
    );
    assert_eq!(
        ok(Dialect::Mysql, "datetime", text("2026-10-20")),
        BindValue::Text("2026-10-20 00:00:00".into())
    );
    assert_eq!(
        ok(
            Dialect::Mysql,
            "datetime",
            SourceValue::DateTime(day.and_hms_milli_opt(8, 5, 9, 120).unwrap())
        ),
        BindValue::Text("2026-10-20 08:05:09.12".into())
    );
    assert_eq!(
        ok(
            Dialect::Postgres,
            "timestamp with time zone",
            text("2026-10-20 08:05:09+08")
        ),
        BindValue::Text("2026-10-20 08:05:09+08".into())
    );
    assert!(
        err(
            Dialect::Postgres,
            "timestamp without time zone",
            text("2026-10-20 08:05:09+08:00")
        )
        .contains("时区")
    );
    assert!(err(Dialect::Mysql, "datetime", text("2026-10-20 25:00:00")).contains("日期时间"));
    assert!(err(Dialect::Mysql, "datetime", text("昨天")).contains("日期时间"));
    assert_eq!(
        ok(Dialect::Mysql, "datetime", text("0000-00-00 00:00:00")),
        BindValue::Text("0000-00-00 00:00:00".into())
    );

    assert_eq!(
        ok(Dialect::Mysql, "time", text("-838:59:59")),
        BindValue::Text("-838:59:59".into())
    );
    assert!(
        err(
            Dialect::Postgres,
            "time without time zone",
            text("-01:00:00")
        )
        .contains("时间")
    );
    assert_eq!(
        ok(
            Dialect::Postgres,
            "time with time zone",
            text("10:00:00+08")
        ),
        BindValue::Text("10:00:00+08".into())
    );
    let excel_time = NaiveDate::from_ymd_opt(1899, 12, 30)
        .unwrap()
        .and_hms_opt(13, 5, 0)
        .unwrap();
    assert_eq!(
        ok(Dialect::Mysql, "time", SourceValue::DateTime(excel_time)),
        BindValue::Text("13:05:00".into())
    );
    assert!(
        err(
            Dialect::Mysql,
            "time",
            SourceValue::DateTime(day.and_hms_opt(1, 0, 0).unwrap())
        )
        .contains("日期时间")
    );
}

#[test]
fn json_enums_sets_and_text_limits() {
    assert_eq!(
        ok(Dialect::Mysql, "json", text("{\"a\":1}")),
        BindValue::Text("{\"a\":1}".into())
    );
    assert_eq!(
        ok(Dialect::Postgres, "jsonb", SourceValue::Number(5.0)),
        BindValue::Text("5".into())
    );
    assert!(err(Dialect::Mysql, "json", text("{oops")).contains("JSON"));

    assert_eq!(
        ok(Dialect::Mysql, "enum('small','big')", text("BIG")),
        BindValue::Text("BIG".into())
    );
    assert!(err(Dialect::Mysql, "enum('small','big')", text("huge")).contains("small、big"));
    assert_eq!(
        ok(Dialect::Mysql, "set('r','w')", text("r,w")),
        BindValue::Text("r,w".into())
    );
    assert_eq!(
        ok(Dialect::Mysql, "set('r','w')", text("")),
        BindValue::Text(String::new())
    );
    assert!(err(Dialect::Mysql, "set('r','w')", text("r,x")).contains("可选值"));

    assert_eq!(
        ok(Dialect::Mysql, "varchar(2)", text("中文")),
        BindValue::Text("中文".into())
    );
    assert!(err(Dialect::Mysql, "varchar(2)", text("中文字")).contains("2 个字符"));
    assert!(err(Dialect::Mysql, "tinytext", text(&"字".repeat(100))).contains("255 字节"));
    assert_eq!(
        ok(Dialect::Mysql, "varchar(20)", SourceValue::Number(42.0)),
        BindValue::Text("42".into())
    );
    assert_eq!(
        ok(Dialect::Mysql, "varchar(20)", SourceValue::Bool(true)),
        BindValue::Text("TRUE".into())
    );
    assert_eq!(
        ok(Dialect::Postgres, "uuid", text("x")),
        BindValue::Text("x".into())
    );
}
