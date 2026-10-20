use super::*;
use crate::table::ValueType;

fn column(data_type: &str, value_type: ValueType) -> ColumnInfo {
    ColumnInfo {
        value_type,
        ..ColumnInfo::new("c", data_type)
    }
}

fn mysql_kind(data_type: &str) -> TargetKind {
    classify(Dialect::Mysql, &column(data_type, ValueType::Text))
}

fn pg_kind(data_type: &str) -> TargetKind {
    classify(Dialect::Postgres, &column(data_type, ValueType::Text))
}

#[test]
fn mysql_integer_ranges_follow_size_and_signedness() {
    assert_eq!(
        mysql_kind("tinyint(4)"),
        TargetKind::Integer {
            min: -128,
            max: 127
        }
    );
    assert_eq!(
        mysql_kind("tinyint(3) unsigned"),
        TargetKind::Integer { min: 0, max: 255 }
    );
    assert_eq!(
        mysql_kind("int unsigned"),
        TargetKind::Integer {
            min: 0,
            max: 4_294_967_295
        }
    );
    assert_eq!(
        mysql_kind("bigint(20) unsigned"),
        TargetKind::Integer {
            min: 0,
            max: i128::from(u64::MAX)
        }
    );
    assert_eq!(
        mysql_kind("mediumint"),
        TargetKind::Integer {
            min: -8_388_608,
            max: 8_388_607
        }
    );
    assert_eq!(mysql_kind("year"), TargetKind::Year);
}

#[test]
fn mysql_decimals_texts_and_lists() {
    assert_eq!(
        mysql_kind("decimal(10,2)"),
        TargetKind::Decimal {
            precision: Some(10),
            scale: Some(2),
            specials: false
        }
    );
    assert_eq!(
        mysql_kind("decimal"),
        TargetKind::Decimal {
            precision: Some(10),
            scale: Some(0),
            specials: false
        }
    );
    assert_eq!(
        mysql_kind("varchar(50)"),
        TargetKind::Text {
            max_chars: Some(50),
            max_bytes: None
        }
    );
    assert_eq!(
        mysql_kind("text"),
        TargetKind::Text {
            max_chars: None,
            max_bytes: Some(65_535)
        }
    );
    assert_eq!(
        mysql_kind("enum('a','it''s','x\\'y')"),
        TargetKind::Enum(vec!["a".into(), "it's".into(), "x'y".into()])
    );
    assert_eq!(
        mysql_kind("set('r','w')"),
        TargetKind::Set(vec!["r".into(), "w".into()])
    );
    assert_eq!(mysql_kind("bit(8)"), TargetKind::Bit { bits: 8 });
    assert_eq!(
        mysql_kind("datetime(6)"),
        TargetKind::DateTime { zoned: false }
    );
    assert_eq!(
        mysql_kind("time"),
        TargetKind::Time {
            zoned: false,
            long: true
        }
    );
    assert_eq!(mysql_kind("longblob"), TargetKind::Binary);
    assert_eq!(
        mysql_kind("double"),
        TargetKind::Float { finite_only: true }
    );
    assert_eq!(mysql_kind("json"), TargetKind::Json);
    assert_eq!(mysql_kind("geometry"), TargetKind::Other);
}

#[test]
fn postgres_types_from_format_type() {
    assert_eq!(
        pg_kind("integer"),
        TargetKind::Integer {
            min: i128::from(i32::MIN),
            max: i128::from(i32::MAX)
        }
    );
    assert_eq!(pg_kind("integer[]"), TargetKind::Other);
    assert_eq!(
        pg_kind("numeric(12,3)"),
        TargetKind::Decimal {
            precision: Some(12),
            scale: Some(3),
            specials: true
        }
    );
    assert_eq!(
        pg_kind("numeric(5)"),
        TargetKind::Decimal {
            precision: Some(5),
            scale: Some(0),
            specials: true
        }
    );
    assert_eq!(
        pg_kind("numeric"),
        TargetKind::Decimal {
            precision: None,
            scale: None,
            specials: true
        }
    );
    assert_eq!(
        pg_kind("timestamp(3) with time zone"),
        TargetKind::DateTime { zoned: true }
    );
    assert_eq!(
        pg_kind("timestamp without time zone"),
        TargetKind::DateTime { zoned: false }
    );
    assert_eq!(
        pg_kind("time with time zone"),
        TargetKind::Time {
            zoned: true,
            long: false
        }
    );
    assert_eq!(
        pg_kind("character varying(20)"),
        TargetKind::Text {
            max_chars: Some(20),
            max_bytes: None
        }
    );
    assert_eq!(
        pg_kind("double precision"),
        TargetKind::Float { finite_only: false }
    );
    assert_eq!(pg_kind("boolean"), TargetKind::Bool);
    assert_eq!(pg_kind("jsonb"), TargetKind::Json);
    assert_eq!(pg_kind("uuid"), TargetKind::Other);
}

#[test]
fn sqlite_affinity_with_dates_and_booleans() {
    let kind =
        |declared: &str, value_type| classify(Dialect::Sqlite, &column(declared, value_type));
    assert_eq!(
        kind("INTEGER", ValueType::Integer { unsigned: false }),
        signed(64)
    );
    assert_eq!(kind("", ValueType::Binary), TargetKind::Other);
    assert_eq!(kind("BLOB", ValueType::Binary), TargetKind::Binary);
    assert_eq!(
        kind("DATETIME", ValueType::Decimal),
        TargetKind::DateTime { zoned: true }
    );
    assert_eq!(kind("DATE", ValueType::Decimal), TargetKind::Date);
    assert_eq!(kind("BOOLEAN", ValueType::Decimal), TargetKind::Bool);
    assert_eq!(
        kind("DECIMAL(10,2)", ValueType::Decimal),
        TargetKind::Decimal {
            precision: None,
            scale: None,
            specials: false
        }
    );
    assert_eq!(kind("MONEY", ValueType::Decimal), TargetKind::Other);
    assert_eq!(
        kind("REAL", ValueType::Float),
        TargetKind::Float { finite_only: true }
    );
}
