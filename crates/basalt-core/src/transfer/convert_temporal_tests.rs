//! 日期、时间、日期时间的转换:和 convert_tests 一样从 [`convert`] 进,按目标列的声明类型分类。

use chrono::NaiveDate;

use super::super::convert::convert;
use crate::sql::Dialect;
use crate::sql::bind::BindValue;
use crate::table::ColumnInfo;
use crate::transfer::BinaryEncoding;
use crate::transfer::source::SourceValue;
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
            "datetime(3)",
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
