//! 全部用临时目录现造的假数据,不碰本机真实的 Navicat 文件。

use super::*;
use crate::config::SslSettings;
use crate::import::SourcePassword;
use crate::query::saved::SavedQueryStore;

fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// 一棵和 Navicat 结构相同的目录树。
fn settings() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "0/0/MySQL/本地/shop/订单统计.sql", b"SELECT 1;");
    write(root, "0/0/MySQL/本地/shop/.DS_Store", b"");
    write(root, "0/0/MySQL/本地/shop/notes.txt", b"");
    write(root, "0/0/MySQL/本地/id_cache.db", b"");
    write(
        root,
        "0/0/MySQL/本地/crm/带BOM.sql",
        "\u{feff}SELECT 2;".as_bytes(),
    );
    write(root, "0/0/MySQL/本地/crm/坏编码.sql", &[0xff, 0xfe]);
    write(root, "0/0/MySQL/只有地址/shop/q.sql", b"SELECT 3;");
    write(root, "0/0/MySQL/查无此人/shop/q.sql", b"SELECT 4;");
    write(
        root,
        "0/0/PostgreSQL/报表库/folio/inventory/馆藏.sql",
        b"SELECT 5;",
    );
    write(
        root,
        "0/0/PostgreSQL/报表库/folio/无schema.sql",
        b"SELECT 6;",
    );
    write(root, "0/0/Redis/缓存/0/x.sql", b"GET x");
    dir
}

fn connection(
    id: &str,
    name: &str,
    driver: DriverKind,
    host: &str,
    user: &str,
) -> ConnectionConfig {
    ConnectionConfig {
        id: id.into(),
        name: name.into(),
        group: None,
        driver,
        endpoint: Endpoint::Network {
            host: host.into(),
            port: driver.default_port().unwrap(),
            username: user.into(),
            database: None,
        },
        save_password: false,
        ssh: None,
        ssl: SslSettings::default(),
    }
}

fn basalt() -> Vec<ConnectionConfig> {
    vec![
        connection("m1", "本地", DriverKind::Mariadb, "127.0.0.1", "root"),
        connection("m2", "生产只读", DriverKind::Mysql, "localhost", "reader"),
        connection("p1", "报表库", DriverKind::Postgres, "10.0.0.8", "folio"),
    ]
}

fn navicat_source(name: &str, host: &str, user: &str) -> SourceConnection {
    SourceConnection {
        id: format!("MySQL:{name}"),
        name: name.into(),
        group: None,
        source_type: "MySQL".into(),
        target: SourceTarget::Supported {
            driver: DriverKind::Mysql,
            endpoint: Endpoint::Network {
                host: host.into(),
                port: 3306,
                username: user.into(),
                database: None,
            },
        },
        password: SourcePassword::Absent,
        ssh: None,
        ssh_secret: SourcePassword::Absent,
        ssl: SslSettings::default(),
        warnings: vec![],
    }
}

fn query(conn_type: &str, connection: &str, schema: Option<&str>) -> NavicatQuery {
    NavicatQuery {
        id: "x".into(),
        conn_type: conn_type.into(),
        connection: connection.into(),
        database: "shop".into(),
        schema: schema.map(str::to_owned),
        name: "q".into(),
        path: PathBuf::from("/nonexistent/q.sql"),
    }
}

#[test]
fn scan_walks_the_navicat_layout() {
    let dir = settings();
    let found = scan_queries(dir.path()).unwrap();
    let ids: Vec<&str> = found.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "0/0/MySQL/只有地址/shop/q.sql",
            "0/0/MySQL/本地/crm/坏编码.sql",
            "0/0/MySQL/本地/crm/带BOM.sql",
            "0/0/MySQL/本地/shop/订单统计.sql",
            "0/0/MySQL/查无此人/shop/q.sql",
            "0/0/PostgreSQL/报表库/folio/无schema.sql",
            "0/0/PostgreSQL/报表库/folio/inventory/馆藏.sql",
            "0/0/Redis/缓存/0/x.sql",
        ]
    );
    let pg = found.iter().find(|q| q.name == "馆藏").unwrap();
    assert_eq!(pg.conn_type, "PostgreSQL");
    assert_eq!(pg.connection, "报表库");
    assert_eq!(pg.database, "folio");
    assert_eq!(pg.schema.as_deref(), Some("inventory"));
}

#[test]
fn scan_reports_a_missing_settings_folder() {
    let dir = tempfile::tempdir().unwrap();
    let error = scan_queries(&dir.path().join("nope")).unwrap_err();
    assert_eq!(error.code(), "storage");
}

#[test]
fn reading_strips_bom_and_rejects_non_utf8() {
    let dir = settings();
    let found = scan_queries(dir.path()).unwrap();
    let by_name = |name: &str| found.iter().find(|q| q.name == name).unwrap();
    assert_eq!(read_query(by_name("带BOM")).unwrap(), "SELECT 2;");
    assert_eq!(
        read_query(by_name("坏编码")).unwrap_err(),
        "不是 UTF-8 文本"
    );
}

#[test]
fn matches_by_name_within_the_same_family() {
    let (target, by) = match_target(&query("MySQL", "本地", None), &basalt(), &[]).unwrap();
    assert_eq!(by, MatchedBy::Name);
    assert_eq!(
        target.connection_id, "m1",
        "Navicat 的 MySQL 连接可以对上 Basalt 的 MariaDB 连接"
    );
    assert_eq!(target.database, "shop");

    let pg = match_target(
        &query("PostgreSQL", "报表库", Some("inventory")),
        &basalt(),
        &[],
    );
    assert_eq!(pg.unwrap().0.schema.as_deref(), Some("inventory"));

    // 同名但类型不同的不算。
    let wrong_type = match_target(&query("PostgreSQL", "本地", None), &basalt(), &[]);
    assert!(
        wrong_type
            .unwrap_err()
            .contains("没有叫「本地」的 PostgreSQL 连接")
    );
}

#[test]
fn falls_back_to_the_navicat_address() {
    let navicat = [navicat_source("只有地址", "127.0.0.1", "reader")];
    let (target, by) =
        match_target(&query("MySQL", "只有地址", None), &basalt(), &navicat).unwrap();
    assert_eq!(by, MatchedBy::Address);
    assert_eq!(
        target.connection_id, "m2",
        "两个本机连接，用户名 reader 对上的是 m2"
    );
}

#[test]
fn ambiguous_or_missing_targets_explain_why() {
    let no_address = match_target(&query("MySQL", "查无此人", None), &basalt(), &[]).unwrap_err();
    assert_eq!(
        no_address,
        "Basalt 里没有叫「查无此人」的 MySQL 连接，Navicat 的连接配置里也找不到它的地址"
    );

    let elsewhere = [navicat_source("查无此人", "10.9.9.9", "root")];
    let no_match =
        match_target(&query("MySQL", "查无此人", None), &basalt(), &elsewhere).unwrap_err();
    assert!(
        no_match.ends_with("也没有地址是 10.9.9.9:3306 的连接"),
        "{no_match}"
    );

    let two_local = [navicat_source("查无此人", "localhost", "nobody")];
    let ambiguous =
        match_target(&query("MySQL", "查无此人", None), &basalt(), &two_local).unwrap_err();
    assert!(
        ambiguous.contains("不止一个（「本地」、「生产只读」）"),
        "{ambiguous}"
    );

    let mut twins = basalt();
    twins.push(connection(
        "m3",
        "本地",
        DriverKind::Mysql,
        "10.0.0.1",
        "root",
    ));
    let same_name = match_target(&query("MySQL", "本地", None), &twins, &[]).unwrap_err();
    assert!(same_name.contains("叫「本地」的连接不止一个"));

    let redis = match_target(&query("Redis", "缓存", None), &basalt(), &[]).unwrap_err();
    assert!(redis.contains("Redis 的查询是命令"), "{redis}");

    let deep = match_target(&query("MySQL", "本地", Some("extra")), &basalt(), &[]).unwrap_err();
    assert!(deep.contains("认不出库名"));
}

#[test]
fn names_basalt_cannot_store_are_sanitized() {
    let mut odd = query("MySQL", "本地", None);
    odd.name = "a:b".into();
    assert_eq!(match_target(&odd, &basalt(), &[]).unwrap().0.name, "a_b");
}

#[test]
fn plan_marks_new_existing_conflicting_and_unmatched() {
    let dir = settings();
    let store_dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(store_dir.path());
    let saved = |db: &str, schema: Option<&str>, name: &str| SavedQueryRef {
        connection_id: if schema.is_some() { "p1" } else { "m1" }.into(),
        database: db.into(),
        schema: schema.map(str::to_owned),
        name: name.into(),
    };
    store
        .save(&saved("shop", None, "订单统计"), "SELECT 1;", false)
        .unwrap();
    store
        .save(
            &saved("folio", Some("inventory"), "馆藏"),
            "SELECT 'mine';",
            false,
        )
        .unwrap();

    let queries = scan_queries(dir.path()).unwrap();
    let plans = plan_queries(&queries, &basalt(), &[], &store);
    let status = |name: &str| {
        let plan = plans.iter().find(|p| p.candidate.name == name).unwrap();
        (plan.candidate.status.clone(), plan.sql.clone())
    };
    assert_eq!(status("订单统计"), (QueryImportStatus::Exists, None));
    assert_eq!(status("馆藏"), (QueryImportStatus::Conflict, None));
    assert_eq!(
        status("带BOM"),
        (QueryImportStatus::New, Some("SELECT 2;".into()))
    );
    assert_eq!(
        status("坏编码").0,
        QueryImportStatus::Unmatched {
            reason: "不是 UTF-8 文本".into()
        }
    );
    let pg_top = plans
        .iter()
        .find(|p| p.candidate.name == "无schema")
        .unwrap();
    assert_eq!(pg_top.candidate.source_type, "PostgreSQL");
    assert_eq!(
        pg_top.candidate.target.as_ref().unwrap().schema.as_deref(),
        Some("public"),
        "库这一层的 PostgreSQL 查询放到 public 下,树上才看得见"
    );
    assert!(matches!(status("x").0, QueryImportStatus::Unmatched { .. }));
}

#[test]
fn import_writes_only_new_queries_and_explains_the_rest() {
    let dir = settings();
    let store_dir = tempfile::tempdir().unwrap();
    let store = SavedQueryStore::new(store_dir.path());
    let queries = scan_queries(dir.path()).unwrap();
    let plans = plan_queries(&queries, &basalt(), &[], &store);
    let ids: Vec<String> = [
        "0/0/MySQL/本地/shop/订单统计.sql",
        "0/0/PostgreSQL/报表库/folio/inventory/馆藏.sql",
        "0/0/MySQL/查无此人/shop/q.sql",
        "0/0/MySQL/已经没了.sql",
    ]
    .map(str::to_owned)
    .to_vec();
    let report = import_planned(&plans, &ids, &store);
    let imported: Vec<&str> = report.imported.iter().map(|q| q.name.as_str()).collect();
    assert_eq!(imported, ["订单统计", "馆藏"]);
    assert_eq!(report.skipped.len(), 2);
    assert!(report.skipped[1].reason.contains("请重新扫描"));
    let pg = SavedQueryRef {
        connection_id: "p1".into(),
        database: "folio".into(),
        schema: Some("inventory".into()),
        name: "馆藏".into(),
    };
    assert_eq!(store.read(&pg).unwrap(), "SELECT 5;");

    // 再导入一次:都已存在,什么都不写。
    let again = plan_queries(&queries, &basalt(), &[], &store);
    let report = import_planned(&again, &ids[..2], &store);
    assert!(report.imported.is_empty());
    assert_eq!(report.skipped[0].reason, "已经导入过了");
}
