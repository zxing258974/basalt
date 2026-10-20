use super::*;

const MYSQL: Flavor = Flavor {
    mariadb: false,
    backslash_escapes: true,
};
const MARIADB: Flavor = Flavor {
    mariadb: true,
    backslash_escapes: true,
};

#[test]
fn mysql_defaults_become_expressions() {
    let d = |raw: &str, ty: &str, extra: &str| column_default(Some(raw.into()), ty, extra, MYSQL);
    assert_eq!(d("abc", "varchar(10)", "").as_deref(), Some("'abc'"));
    assert_eq!(d("it's", "varchar(10)", "").as_deref(), Some("'it''s'"));
    assert_eq!(d("0", "int unsigned", "").as_deref(), Some("0"));
    assert_eq!(d("1.50", "decimal(5,2)", "").as_deref(), Some("1.50"));
    assert_eq!(
        d("CURRENT_TIMESTAMP", "timestamp", "").as_deref(),
        Some("CURRENT_TIMESTAMP")
    );
    assert_eq!(
        d("CURRENT_TIMESTAMP(3)", "datetime(3)", "DEFAULT_GENERATED").as_deref(),
        Some("CURRENT_TIMESTAMP(3)")
    );
    assert_eq!(
        d("rand()", "double", "default_generated").as_deref(),
        Some("(rand())")
    );
    assert_eq!(
        d(r"_utf8mb4\'[]\'", "json", "default_generated").as_deref(),
        Some("(_utf8mb4'[]')")
    );
    assert_eq!(d("b'101'", "bit(3)", "").as_deref(), Some("b'101'"));
    assert_eq!(d("2026-01-01", "date", "").as_deref(), Some("'2026-01-01'"));
    assert_eq!(column_default(None, "int", "", MYSQL), None);
}

#[test]
fn mariadb_defaults_are_already_expressions() {
    let d = |raw: &str| column_default(Some(raw.into()), "varchar(10)", "", MARIADB);
    assert_eq!(d("'abc'").as_deref(), Some("'abc'"));
    assert_eq!(d("NULL"), None);
    assert_eq!(
        d("current_timestamp()").as_deref(),
        Some("current_timestamp()")
    );
}

#[test]
fn on_update_and_charset_come_from_extra_and_collation() {
    assert_eq!(
        on_update("DEFAULT_GENERATED on update CURRENT_TIMESTAMP(3)").as_deref(),
        Some("CURRENT_TIMESTAMP(3)")
    );
    assert_eq!(on_update("auto_increment"), None);
    assert_eq!(charset_of("utf8mb4_0900_ai_ci"), "utf8mb4");
    assert_eq!(charset_of("binary"), "binary");
    assert!(!is_current_timestamp("now(x)"));
}

#[test]
fn partial_failure_names_what_took_effect() {
    let error = || CoreError::Database("boom".into());
    let first = partial_failure(0, 2, error()).to_string();
    assert!(first.contains("没有任何改动生效") && first.contains("后面的语句也没有执行"));
    let second = partial_failure(1, 2, error());
    assert_eq!(second.code(), "partially_applied");
    let text = second.to_string();
    assert!(text.contains("第 2 条语句执行失败：boom"), "{text}");
    assert!(text.contains("已经生效：第 1 条"), "{text}");
    assert!(text.contains("没有生效：第 2 条。请重新读取"), "{text}");
    let middle = partial_failure(2, 5, error()).to_string();
    assert!(middle.contains("已经生效：第 1–2 条"), "{middle}");
    assert!(
        middle.contains("没有生效：第 3 条。第 4–5 条没有执行。"),
        "{middle}"
    );
}

#[test]
fn lost_connections_leave_the_failed_statement_uncertain() {
    let lost = || CoreError::Network("连接被重置".into());
    let first = partial_failure(0, 2, lost());
    assert_eq!(first.code(), "partially_applied");
    let text = first.to_string();
    assert!(
        text.contains("已经生效：没有。第 1 条无法确定是否生效。第 2 条没有执行。"),
        "{text}"
    );
    let later = partial_failure(1, 2, CoreError::Timeout).to_string();
    assert!(
        later.contains("已经生效：第 1 条")
            && later.contains("第 2 条无法确定是否生效。请重新读取"),
        "{later}"
    );
}
