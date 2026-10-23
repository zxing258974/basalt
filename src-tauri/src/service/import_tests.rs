//! 导入连接的 service 测试:合成的 conn.plist / .ncx 夹具 + 内存密码存储 + 临时配置目录。

use basalt_core::config::SslSettings;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use basalt_core::CoreError;
use basalt_core::config::{ConnectionDraft, DriverKind, Endpoint};
use basalt_core::import::CandidateStatus;
use basalt_core::import::navicat::NavicatLocation;
use basalt_core::secret::{MemorySecretStore, SecretStore, password_key, ssh_secret_key};

use super::*;
use crate::service;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../crates/basalt-core/tests/fixtures/navicat"
);
/// .ncx 夹具里加密的假密码。
const ORDER_PASSWORD: &str = "Fake-Pass#2026";
const REPORT_PASSWORD: &str = "测试密码";
/// 「跳板机后的库」的 SSH 密码。
const SSH_PASSWORD: &str = "sixteen-chars-pw";

struct Harness {
    dir: tempfile::TempDir,
    secrets: Arc<MemorySecretStore>,
    state: AppState,
    local: NavicatLocation,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let state = AppState::new(dir.path().join("config/connections.json"), secrets.clone());
    let local = NavicatLocation::under_home(&dir.path().join("home"));
    Harness {
        dir,
        secrets,
        state,
        local,
    }
}

impl Harness {
    fn install_local_navicat(&self) {
        std::fs::create_dir_all(self.local.conn_plist.parent().unwrap()).unwrap();
        std::fs::copy(
            Path::new(FIXTURES).join("conn.plist"),
            &self.local.conn_plist,
        )
        .unwrap();
    }

    fn ncx(&self) -> ImportSource {
        ImportSource::NcxFile {
            path: format!("{FIXTURES}/export.ncx"),
        }
    }

    fn write_ncx(&self, body: &str) -> ImportSource {
        let path: PathBuf = self.dir.path().join("custom.ncx");
        std::fs::write(&path, format!("<Connections>{body}</Connections>")).unwrap();
        ImportSource::NcxFile {
            path: path.to_string_lossy().into_owned(),
        }
    }

    fn config_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("config/connections.json")).unwrap_or_default()
    }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

fn mysql_draft(name: &str, host: &str, username: &str) -> ConnectionDraft {
    ConnectionDraft {
        id: None,
        name: name.into(),
        group: None,
        driver: DriverKind::Mysql,
        endpoint: Endpoint::Network {
            host: host.into(),
            port: 3306,
            username: username.into(),
            database: None,
        },
        save_password: false,
        password: None,
        ssh: None,
        ssl: SslSettings::default(),
        ssh_secret: None,
    }
}

#[tokio::test]
async fn scanning_local_navicat_marks_existing_and_unsupported_connections() {
    let h = harness();
    h.install_local_navicat();
    service::save_connection(&h.state, mysql_draft("我的本地库", "127.0.0.1", "root"))
        .await
        .unwrap();

    let candidates = scan_import_source(&h.state, &ImportSource::LocalNavicat, Some(&h.local))
        .await
        .unwrap();
    let statuses: Vec<_> = candidates
        .iter()
        .map(|c| (c.name.as_str(), &c.status))
        .collect();
    assert_eq!(statuses.len(), 4);
    assert_eq!(
        statuses[0],
        (
            "本地 MySQL",
            &CandidateStatus::Existing {
                existing_name: "我的本地库".into()
            }
        )
    );
    assert_eq!(statuses[1].1, &CandidateStatus::New);
    assert_eq!(statuses[3], ("会话缓存", &CandidateStatus::New));
    assert_eq!(candidates[3].driver, Some(DriverKind::Redis));
}

#[tokio::test]
async fn scanning_without_navicat_installed_reports_not_found() {
    let h = harness();
    let error = scan_import_source(&h.state, &ImportSource::LocalNavicat, Some(&h.local))
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage");
}

#[tokio::test]
async fn scan_results_never_carry_plaintext_passwords() {
    let h = harness();
    let candidates = scan_import_source(&h.state, &h.ncx(), Some(&h.local))
        .await
        .unwrap();
    let json = serde_json::to_string(&candidates).unwrap();
    for secret in [ORDER_PASSWORD, REPORT_PASSWORD, "7CD6F821", SSH_PASSWORD] {
        assert!(!json.contains(secret), "{json}");
    }
    assert_eq!(candidates.len(), 10);
    let tunneled = candidates
        .iter()
        .find(|c| c.name == "跳板机后的库")
        .unwrap();
    assert_eq!(
        tunneled.ssh.as_ref().map(|t| t.host.as_str()),
        Some("bastion.example.test")
    );
}

#[tokio::test]
async fn importing_a_tunneled_connection_keeps_its_ssh_password_in_the_keychain() {
    let h = harness();
    let report = import_connections(
        &h.state,
        &h.ncx(),
        &ids(&["MARIADB:跳板机后的库"]),
        Some(&h.local),
    )
    .await
    .unwrap();
    let item = &report.imported[0];
    assert!(item.tunneled && item.ssh_secret_saved && !item.password_saved);
    let config = h.state.find(&item.id).unwrap();
    let tunnel = config.ssh.clone().unwrap();
    assert_eq!(
        (tunnel.host.as_str(), tunnel.username.as_str()),
        ("bastion.example.test", "jump")
    );
    assert!(tunnel.save_secret);
    assert_eq!(
        h.secrets.get(&ssh_secret_key(&item.id)).unwrap().as_deref(),
        Some(SSH_PASSWORD)
    );
    assert!(
        !h.config_text().contains(SSH_PASSWORD),
        "配置文件里没有明文"
    );
}

#[tokio::test]
async fn importing_from_ncx_stores_passwords_in_the_keychain_only() {
    let h = harness();
    let selected = ids(&[
        "MYSQL:订单库",
        "POSTGRESQL:报表 PG",
        "MARIADB:跳板机后的库",
        "MYSQL:旧版导出",
        "REDIS:缓存",
        "MYSQL:不存在",
    ]);
    let report = import_connections(&h.state, &h.ncx(), &selected, Some(&h.local))
        .await
        .unwrap();

    let imported: Vec<_> = report
        .imported
        .iter()
        .map(|c| (c.name.as_str(), c.password_saved))
        .collect();
    assert_eq!(
        imported,
        [
            ("订单库", true),
            ("报表 PG", true),
            ("跳板机后的库", false),
            ("旧版导出", false),
            ("缓存", true)
        ]
    );
    let skipped: Vec<_> = report.skipped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skipped, ["MYSQL:不存在"]);

    let saved = service::list_connections(&h.state).await.unwrap();
    let save_flags: Vec<_> = saved
        .iter()
        .map(|c| (c.config.name.as_str(), c.config.save_password))
        .collect();
    assert_eq!(
        save_flags,
        [
            ("订单库", true),
            ("报表 PG", true),
            ("跳板机后的库", false),
            ("旧版导出", false),
            ("缓存", true)
        ],
        "只有带着密码的连接标为保存密码"
    );
    let secret = |name: &str| {
        let id = &report.imported.iter().find(|c| c.name == name).unwrap().id;
        h.secrets.get(&password_key(id)).unwrap()
    };
    assert_eq!(secret("订单库").as_deref(), Some(ORDER_PASSWORD));
    assert_eq!(secret("报表 PG").as_deref(), Some(REPORT_PASSWORD));
    assert_eq!(secret("跳板机后的库"), None);
    assert_eq!(secret("旧版导出"), None, "解不开的密码不写,首次打开时再输");

    let report_json = serde_json::to_string(&report).unwrap();
    let config = h.config_text();
    for text in [&report_json, &config] {
        assert!(
            !text.contains(ORDER_PASSWORD) && !text.contains(REPORT_PASSWORD),
            "{text}"
        );
    }
}

#[tokio::test]
async fn imported_sqlite_connection_opens() {
    let h = harness();
    let db = h.dir.path().join("notes.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db)
        .create_if_missing(true);
    sqlx::SqlitePool::connect_with(options)
        .await
        .unwrap()
        .close()
        .await;
    let source = h.write_ncx(&format!(
        r#"<Connection ConnectionName="笔记" ConnType="SQLITE" DatabaseFileName="{}"/>"#,
        db.display()
    ));

    let report = import_connections(&h.state, &source, &ids(&["SQLITE:笔记"]), Some(&h.local))
        .await
        .unwrap();
    assert_eq!(report.imported[0].driver, DriverKind::Sqlite);
    let id = &report.imported[0].id;
    let server = service::open_connection(&h.state, id, None, None)
        .await
        .unwrap();
    assert_eq!(server.product, "SQLite");
}

#[tokio::test]
async fn selecting_nothing_importable_does_not_touch_the_config() {
    let h = harness();
    let report = import_connections(
        &h.state,
        &h.ncx(),
        &ids(&["SQLSERVER:财务"]),
        Some(&h.local),
    )
    .await
    .unwrap();
    assert!(report.imported.is_empty());
    assert_eq!(report.skipped.len(), 1);
    assert!(!h.dir.path().join("config/connections.json").exists());
}

/// 钥匙串写入一律失败,模拟用户在钥匙串弹窗里点了「拒绝」。
struct LockedKeychain;

impl SecretStore for LockedKeychain {
    fn get(&self, _key: &str) -> basalt_core::CoreResult<Option<String>> {
        Ok(None)
    }
    fn set(&self, _key: &str, _secret: &str) -> basalt_core::CoreResult<()> {
        Err(CoreError::Secret("用户拒绝访问钥匙串".into()))
    }
    fn delete(&self, _key: &str) -> basalt_core::CoreResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn keychain_failure_leaves_no_imported_connections() {
    let h = harness();
    let state = AppState::new(
        h.dir.path().join("locked/connections.json"),
        Arc::new(LockedKeychain),
    );
    let error = import_connections(&state, &h.ncx(), &ids(&["MYSQL:订单库"]), Some(&h.local))
        .await
        .unwrap_err();
    assert_eq!(error.code, "secret");
    assert!(service::list_connections(&state).await.unwrap().is_empty());
}

/// 记下写过哪些账户,用来检查回滚后一个都不剩。
#[cfg(unix)]
#[derive(Default)]
struct RecordingKeychain {
    inner: MemorySecretStore,
    written: std::sync::Mutex<Vec<String>>,
}

#[cfg(unix)]
impl SecretStore for RecordingKeychain {
    fn get(&self, key: &str) -> basalt_core::CoreResult<Option<String>> {
        self.inner.get(key)
    }
    fn set(&self, key: &str, secret: &str) -> basalt_core::CoreResult<()> {
        self.written.lock().unwrap().push(key.to_owned());
        self.inner.set(key, secret)
    }
    fn delete(&self, key: &str) -> basalt_core::CoreResult<()> {
        self.inner.delete(key)
    }
}

// 用只读目录造写入失败:Windows 的只读属性挡不住在目录里建文件,只在 Unix 上跑。
#[cfg(unix)]
#[tokio::test]
async fn config_write_failure_removes_the_passwords_just_written() {
    use std::os::unix::fs::PermissionsExt;

    let h = harness();
    let keychain = Arc::new(RecordingKeychain::default());
    let state = AppState::new(
        h.dir.path().join("config/connections.json"),
        keychain.clone(),
    );
    service::save_connection(&state, mysql_draft("已有", "db.example.test", "u"))
        .await
        .unwrap();
    let config_dir = state.config_dir().to_path_buf();
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let selected = ids(&["MYSQL:订单库", "POSTGRESQL:报表 PG"]);
    let result = import_connections(&state, &h.ncx(), &selected, Some(&h.local)).await;
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(result.unwrap_err().code, "storage");
    assert_eq!(service::list_connections(&state).await.unwrap().len(), 1);
    let written = keychain.written.lock().unwrap().clone();
    assert_eq!(written.len(), 2, "两个带密码的连接都先写了钥匙串");
    for key in written {
        assert_eq!(keychain.get(&key).unwrap(), None, "{key} 应该已回滚");
    }
}

#[tokio::test]
async fn ncx_import_does_not_need_the_home_folder() {
    let h = harness();
    let report = import_connections(&h.state, &h.ncx(), &ids(&["MYSQL:订单库"]), None)
        .await
        .unwrap();
    assert_eq!(report.imported.len(), 1);

    // 没有主目录:macOS 上是找不到目录;Windows 上本机来源本来就不支持。
    let error = scan_import_source(&h.state, &ImportSource::LocalNavicat, None)
        .await
        .unwrap_err();
    let expected = if cfg!(target_os = "macos") {
        "storage"
    } else {
        "unsupported"
    };
    assert_eq!(error.code, expected);
}

/// 第二次写钥匙串时失败,模拟用户在第二个弹窗里点了「拒绝」。
#[derive(Default)]
struct FlakyKeychain {
    inner: MemorySecretStore,
    written: std::sync::Mutex<Vec<String>>,
}

impl SecretStore for FlakyKeychain {
    fn get(&self, key: &str) -> basalt_core::CoreResult<Option<String>> {
        self.inner.get(key)
    }
    fn set(&self, key: &str, secret: &str) -> basalt_core::CoreResult<()> {
        let mut written = self.written.lock().unwrap();
        if written.len() == 1 {
            return Err(CoreError::Secret("用户拒绝访问钥匙串".into()));
        }
        written.push(key.to_owned());
        self.inner.set(key, secret)
    }
    fn delete(&self, key: &str) -> basalt_core::CoreResult<()> {
        self.inner.delete(key)
    }
}

#[tokio::test]
async fn keychain_failure_midway_removes_passwords_already_written() {
    let h = harness();
    let keychain = Arc::new(FlakyKeychain::default());
    let state = AppState::new(
        h.dir.path().join("flaky/connections.json"),
        keychain.clone(),
    );
    let selected = ids(&["MYSQL:订单库", "POSTGRESQL:报表 PG"]);

    let error = import_connections(&state, &h.ncx(), &selected, Some(&h.local))
        .await
        .unwrap_err();
    assert_eq!(error.code, "secret");
    assert!(service::list_connections(&state).await.unwrap().is_empty());
    let written = keychain.written.lock().unwrap().clone();
    assert_eq!(written.len(), 1, "第一个密码已经写进去了");
    assert_eq!(keychain.get(&written[0]).unwrap(), None, "应该已回滚");
}

#[tokio::test]
async fn ncx_fills_the_password_of_a_connection_imported_without_one() {
    let h = harness();
    // 先前从本机 Navicat 导入、没带密码的连接:地址和 .ncx 里的「订单库」一致,名字不同。
    let existing = service::save_connection(
        &h.state,
        mysql_draft("订单库-本机", "mysql.example.test", "app_user"),
    )
    .await
    .unwrap();

    let candidates = scan_import_source(&h.state, &h.ncx(), Some(&h.local))
        .await
        .unwrap();
    let order = candidates.iter().find(|c| c.id == "MYSQL:订单库").unwrap();
    assert_eq!(
        order.status,
        CandidateStatus::FillPassword {
            existing_id: existing.id.clone(),
            existing_name: "订单库-本机".into(),
        }
    );

    let report = import_connections(&h.state, &h.ncx(), &ids(&["MYSQL:订单库"]), Some(&h.local))
        .await
        .unwrap();
    assert!(report.imported.is_empty(), "补密码不应新建连接");
    let filled: Vec<_> = report
        .password_filled
        .iter()
        .map(|c| (c.id.as_str(), c.name.as_str()))
        .collect();
    assert_eq!(filled, [(existing.id.as_str(), "订单库-本机")]);

    let saved = service::list_connections(&h.state).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].config.save_password);
    assert_eq!(
        h.secrets
            .get(&password_key(&existing.id))
            .unwrap()
            .as_deref(),
        Some(ORDER_PASSWORD)
    );
    let report_json = serde_json::to_string(&report).unwrap();
    for text in [&report_json, &h.config_text()] {
        assert!(!text.contains(ORDER_PASSWORD), "{text}");
    }
}

// 用只读目录造写入失败:Windows 的只读属性挡不住在目录里建文件,只在 Unix 上跑。
#[cfg(unix)]
#[tokio::test]
async fn config_write_failure_while_filling_restores_the_keychain() {
    use std::os::unix::fs::PermissionsExt;

    let h = harness();
    let existing = service::save_connection(
        &h.state,
        mysql_draft("订单库-本机", "mysql.example.test", "app_user"),
    )
    .await
    .unwrap();
    let config_dir = h.dir.path().join("config");
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let result =
        import_connections(&h.state, &h.ncx(), &ids(&["MYSQL:订单库"]), Some(&h.local)).await;
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(result.unwrap_err().code, "storage");
    assert_eq!(h.secrets.get(&password_key(&existing.id)).unwrap(), None);
    assert!(
        !service::list_connections(&h.state).await.unwrap()[0]
            .config
            .save_password
    );
}
