//! 导入 Navicat 查询的 service 测试:临时「主目录」里现造的 Navicat 查询目录 + 合成的 conn.plist。

use std::path::Path;
use std::sync::Arc;

use basalt_core::config::{ConnectionDraft, DriverKind, Endpoint};
use basalt_core::import::navicat::queries::{MatchedBy, QueryImportStatus};
use basalt_core::query::saved::SavedQueryRef;
use basalt_core::secret::MemorySecretStore;

use super::*;
use crate::service;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../crates/basalt-core/tests/fixtures/navicat"
);

struct Harness {
    _dir: tempfile::TempDir,
    state: AppState,
    local: NavicatLocation,
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        dir.path().join("config/connections.json"),
        Arc::new(MemorySecretStore::default()),
    );
    let local = NavicatLocation::under_home(&dir.path().join("home"));
    // conn.plist 里有「测试 MySQL」(mysql.example.test:13306)。
    std::fs::create_dir_all(local.conn_plist.parent().unwrap()).unwrap();
    std::fs::copy(Path::new(FIXTURES).join("conn.plist"), &local.conn_plist).unwrap();
    write(
        &local.settings,
        "0/0/MySQL/本地 MySQL/shop/日报.sql",
        "SELECT 1;",
    );
    write(
        &local.settings,
        "0/0/MySQL/测试 MySQL/qa/回归.sql",
        "SELECT 2;",
    );
    write(
        &local.settings,
        "0/0/PostgreSQL/分析库/warehouse/mart/汇总.sql",
        "SELECT 3;",
    );
    Harness {
        _dir: dir,
        state,
        local,
    }
}

fn mysql(name: &str, host: &str, port: u16) -> ConnectionDraft {
    ConnectionDraft {
        id: None,
        name: name.into(),
        group: None,
        driver: DriverKind::Mysql,
        endpoint: Endpoint::Network {
            host: host.into(),
            port,
            username: "tester".into(),
            database: None,
        },
        save_password: false,
        password: None,
    }
}

#[tokio::test]
async fn preview_matches_by_name_then_by_navicat_address() {
    let h = harness();
    let by_name = service::save_connection(&h.state, mysql("本地 MySQL", "127.0.0.1", 3306))
        .await
        .unwrap();
    let by_address =
        service::save_connection(&h.state, mysql("QA 库", "mysql.example.test", 13306))
            .await
            .unwrap();

    let candidates = scan_navicat_queries(&h.state, Some(&h.local))
        .await
        .unwrap();
    assert_eq!(candidates.len(), 3);
    let find = |name: &str| candidates.iter().find(|c| c.name == name).unwrap();

    let daily = find("日报");
    assert_eq!(daily.status, QueryImportStatus::New);
    assert_eq!(daily.matched_by, Some(MatchedBy::Name));
    assert_eq!(daily.target.as_ref().unwrap().connection_id, by_name.id);

    let regression = find("回归");
    assert_eq!(regression.matched_by, Some(MatchedBy::Address));
    let target = regression.target.as_ref().unwrap();
    assert_eq!(target.connection_id, by_address.id);
    assert_eq!(target.database, "qa");

    let summary = find("汇总");
    assert_eq!(summary.source_schema.as_deref(), Some("mart"));
    assert!(
        matches!(&summary.status, QueryImportStatus::Unmatched { reason } if reason.contains("分析库"))
    );
}

#[tokio::test]
async fn import_writes_selected_new_queries_only() {
    let h = harness();
    let connection = service::save_connection(&h.state, mysql("本地 MySQL", "127.0.0.1", 3306))
        .await
        .unwrap();
    let ids = vec![
        "0/0/MySQL/本地 MySQL/shop/日报.sql".to_owned(),
        "0/0/PostgreSQL/分析库/warehouse/mart/汇总.sql".to_owned(),
    ];
    let report = import_navicat_queries(&h.state, &ids, Some(&h.local))
        .await
        .unwrap();
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.skipped.len(), 1);
    let saved = SavedQueryRef {
        connection_id: connection.id.clone(),
        database: "shop".into(),
        schema: None,
        name: "日报".into(),
    };
    assert_eq!(h.state.saved_queries.read(&saved).unwrap(), "SELECT 1;");
    // Navicat 的文件原样不动。
    assert_eq!(
        std::fs::read_to_string(h.local.settings.join("0/0/MySQL/本地 MySQL/shop/日报.sql"))
            .unwrap(),
        "SELECT 1;"
    );

    let again = scan_navicat_queries(&h.state, Some(&h.local))
        .await
        .unwrap();
    let daily = again.iter().find(|c| c.name == "日报").unwrap();
    assert_eq!(daily.status, QueryImportStatus::Exists);
}

#[tokio::test]
async fn missing_navicat_folders_are_reported() {
    let h = harness();
    std::fs::remove_dir_all(&h.local.settings).unwrap();
    let error = scan_navicat_queries(&h.state, Some(&h.local))
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage");
    let error = scan_navicat_queries(&h.state, None).await.unwrap_err();
    assert_eq!(error.code, "storage");
}

#[tokio::test]
async fn without_conn_plist_only_names_match() {
    let h = harness();
    std::fs::remove_file(&h.local.conn_plist).unwrap();
    service::save_connection(&h.state, mysql("QA 库", "mysql.example.test", 13306))
        .await
        .unwrap();
    let candidates = scan_navicat_queries(&h.state, Some(&h.local))
        .await
        .unwrap();
    let regression = candidates.iter().find(|c| c.name == "回归").unwrap();
    assert!(
        matches!(&regression.status, QueryImportStatus::Unmatched { reason } if reason.contains("找不到它的地址"))
    );
}
