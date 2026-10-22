//! service 层测试:内存密码存储 + 临时配置目录 + 真实 SQLite 文件,
//! 设置 BASALT_IT 时额外连 docker-compose.test.yml 里的 PostgreSQL。

use basalt_core::config::SslSettings;
use std::sync::Arc;

use basalt_core::config::{ConnectionDraft, DriverKind, Endpoint};
use basalt_core::secret::{MemorySecretStore, QuietDelete, SecretStore, password_key};

use crate::service;
use crate::state::AppState;

/// 本机一次性测试容器的密码,与 docker-compose.test.yml 一致。
const TEST_PASSWORD: &str = "basalt-test";

struct Harness {
    _dir: tempfile::TempDir,
    sqlite_path: String,
    secrets: Arc<MemorySecretStore>,
    state: AppState,
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let sqlite_path = dir.path().join("app.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&sqlite_path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, title TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let secrets = Arc::new(MemorySecretStore::default());
    let state = AppState::new(dir.path().join("config/connections.json"), secrets.clone());
    Harness {
        sqlite_path: sqlite_path.to_string_lossy().into_owned(),
        _dir: dir,
        secrets,
        state,
    }
}

fn sqlite_draft(path: &str) -> ConnectionDraft {
    ConnectionDraft {
        id: None,
        name: "本地文件".into(),
        group: None,
        driver: DriverKind::Sqlite,
        endpoint: Endpoint::File { path: path.into() },
        save_password: false,
        password: None,
        ssh: None,
        ssl: SslSettings::default(),
        ssh_secret: None,
    }
}

fn pg_draft(password: Option<&str>, save_password: bool) -> ConnectionDraft {
    ConnectionDraft {
        id: None,
        name: "测试 PG".into(),
        group: Some("测试".into()),
        driver: DriverKind::Postgres,
        endpoint: Endpoint::Network {
            host: "127.0.0.1".into(),
            port: 25432,
            username: "postgres".into(),
            database: None,
        },
        save_password,
        password: password.map(str::to_owned),
        ssh: None,
        ssl: SslSettings::default(),
        ssh_secret: None,
    }
}

fn pg_enabled() -> bool {
    matches!(std::env::var("BASALT_IT").as_deref(), Ok("1" | "all"))
        || std::env::var("BASALT_IT").is_ok_and(|v| v.split(',').any(|t| t.trim() == "postgres"))
}

#[tokio::test]
async fn saves_lists_and_opens_a_sqlite_connection() {
    let h = harness().await;
    let saved = service::save_connection(&h.state, sqlite_draft(&h.sqlite_path))
        .await
        .unwrap();
    assert!(!saved.id.is_empty());

    let listed = service::list_connections(&h.state).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].is_open);

    let server = service::open_connection(&h.state, &saved.id, None, None)
        .await
        .unwrap();
    assert_eq!(server.product, "SQLite");
    assert!(service::list_connections(&h.state).await.unwrap()[0].is_open);

    let databases = service::list_databases(&h.state, &saved.id).await.unwrap();
    assert_eq!(databases, ["main"]);
    let tables = service::list_tables(&h.state, &saved.id, "main", None)
        .await
        .unwrap();
    assert_eq!(
        tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["items"]
    );

    service::close_connection(&h.state, &saved.id).await;
    let error = service::list_databases(&h.state, &saved.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_connected");
}

#[tokio::test]
async fn invalid_draft_reports_field_errors() {
    let h = harness().await;
    let error = service::save_connection(&h.state, sqlite_draft("relative.db"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_config");
    assert_eq!(error.fields[0].field, "path");
    assert!(
        service::list_connections(&h.state)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn password_is_kept_replaced_or_removed_according_to_draft() {
    let h = harness().await;
    let saved = service::save_connection(&h.state, pg_draft(Some("first"), true))
        .await
        .unwrap();
    let key = password_key(&saved.id);
    assert_eq!(h.secrets.get(&key).unwrap().as_deref(), Some("first"));

    // 编辑但没动密码框:沿用。
    let edit = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..pg_draft(None, true)
    };
    service::save_connection(&h.state, edit).await.unwrap();
    assert_eq!(h.secrets.get(&key).unwrap().as_deref(), Some("first"));

    // 填了新密码:替换。
    let edit = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..pg_draft(Some("second"), true)
    };
    service::save_connection(&h.state, edit).await.unwrap();
    assert_eq!(h.secrets.get(&key).unwrap().as_deref(), Some("second"));

    // 取消「保存密码」:删除。
    let edit = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..pg_draft(None, false)
    };
    service::save_connection(&h.state, edit).await.unwrap();
    assert_eq!(h.secrets.get(&key).unwrap(), None);
}

#[tokio::test]
async fn duplicate_copies_password_and_delete_cleans_up() {
    let h = harness().await;
    let saved = service::save_connection(&h.state, pg_draft(Some("pw"), true))
        .await
        .unwrap();
    let copy = service::duplicate_connection(&h.state, &saved.id)
        .await
        .unwrap();
    assert_eq!(copy.name, "测试 PG 副本");
    assert_eq!(
        h.secrets.get(&password_key(&copy.id)).unwrap().as_deref(),
        Some("pw")
    );

    let deleted = service::delete_connection(&h.state, &saved.id)
        .await
        .unwrap();
    assert!(!deleted.password_left);
    assert_eq!(h.secrets.get(&password_key(&saved.id)).unwrap(), None);
    let remaining: Vec<_> = service::list_connections(&h.state)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.config.id)
        .collect();
    assert_eq!(remaining, [copy.id]);

    let error = service::delete_connection(&h.state, &saved.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, "connection_not_found");
}

#[tokio::test]
async fn renaming_keeps_session_but_changing_endpoint_closes_it() {
    let h = harness().await;
    let saved = service::save_connection(&h.state, sqlite_draft(&h.sqlite_path))
        .await
        .unwrap();
    service::open_connection(&h.state, &saved.id, None, None)
        .await
        .unwrap();

    let rename = ConnectionDraft {
        id: Some(saved.id.clone()),
        name: "改名".into(),
        group: Some("归档".into()),
        ..sqlite_draft(&h.sqlite_path)
    };
    service::save_connection(&h.state, rename).await.unwrap();
    assert!(
        service::list_connections(&h.state).await.unwrap()[0].is_open,
        "只改名和分组不该断开连接"
    );

    let other_file = format!("{}.copy", h.sqlite_path);
    std::fs::copy(&h.sqlite_path, &other_file).unwrap();
    let repoint = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..sqlite_draft(&other_file)
    };
    service::save_connection(&h.state, repoint).await.unwrap();
    assert!(!service::list_connections(&h.state).await.unwrap()[0].is_open);
}

#[tokio::test]
async fn saving_with_unknown_id_does_not_resurrect_a_deleted_connection() {
    let h = harness().await;
    let saved = service::save_connection(&h.state, sqlite_draft(&h.sqlite_path))
        .await
        .unwrap();
    service::delete_connection(&h.state, &saved.id)
        .await
        .unwrap();

    let stale_edit = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..sqlite_draft(&h.sqlite_path)
    };
    let error = service::save_connection(&h.state, stale_edit)
        .await
        .unwrap_err();
    assert_eq!(error.code, "connection_not_found");
    assert!(
        service::list_connections(&h.state)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn remember_password_saves_secret_without_dropping_the_session() {
    // 密码框流程:先用临时输入的密码打开成功,再记住它。这里用已打开的 SQLite 会话
    // 验证「记住」不会断开刚建好的连接。
    let h = harness().await;
    let saved = service::save_connection(&h.state, sqlite_draft(&h.sqlite_path))
        .await
        .unwrap();
    service::open_connection(&h.state, &saved.id, None, None)
        .await
        .unwrap();

    service::remember_password(&h.state, &saved.id, "typed-in".into())
        .await
        .unwrap();
    assert_eq!(
        h.secrets.get(&password_key(&saved.id)).unwrap().as_deref(),
        Some("typed-in")
    );
    let listed = service::list_connections(&h.state).await.unwrap();
    assert!(listed[0].config.save_password);
    assert!(listed[0].is_open, "记住密码不应断开已打开的连接");

    let error = service::remember_password(&h.state, "missing", "x".into())
        .await
        .unwrap_err();
    assert_eq!(error.code, "connection_not_found");
}

/// 钥匙串读写一律失败,模拟用户在钥匙串弹窗里点了「拒绝」。
struct LockedKeychain;

impl SecretStore for LockedKeychain {
    fn get(&self, _key: &str) -> basalt_core::CoreResult<Option<String>> {
        Ok(None)
    }
    fn set(&self, _key: &str, _secret: &str) -> basalt_core::CoreResult<()> {
        Err(basalt_core::CoreError::Secret("用户拒绝访问钥匙串".into()))
    }
    fn delete(&self, _key: &str) -> basalt_core::CoreResult<()> {
        Err(basalt_core::CoreError::Secret("用户拒绝访问钥匙串".into()))
    }
}

#[tokio::test]
async fn keychain_failure_on_save_leaves_no_half_written_connection() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        dir.path().join("connections.json"),
        Arc::new(LockedKeychain),
    );

    let error = service::save_connection(&state, pg_draft(Some("pw"), true))
        .await
        .unwrap_err();
    assert_eq!(error.code, "secret");
    assert!(
        service::list_connections(&state).await.unwrap().is_empty(),
        "钥匙串写失败时不应留下配置,否则重试会产生重复连接"
    );
}

#[tokio::test]
async fn keychain_failure_on_delete_still_removes_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        dir.path().join("connections.json"),
        Arc::new(LockedKeychain),
    );
    let path = dir.path().join("none.db").display().to_string();
    let saved = service::save_connection(&state, sqlite_draft(&path))
        .await
        .unwrap();

    let deleted = service::delete_connection(&state, &saved.id).await.unwrap();
    assert!(deleted.password_left, "钥匙串出错时密码可能还在,要如实报告");
    assert!(service::list_connections(&state).await.unwrap().is_empty());
}

/// 密码是旧版 Basalt 存的:要授权才能删。会弹框的 `delete` 一被调用就算测试失败。
/// 一读就算测试失败:没存密码的连接,保存、打开、测试、复制都不该去读钥匙串(读就可能弹授权框)。
struct UnreadableKeychain;

impl SecretStore for UnreadableKeychain {
    fn get(&self, key: &str) -> basalt_core::CoreResult<Option<String>> {
        panic!("不该读钥匙串:{key}");
    }
    fn set(&self, key: &str, _secret: &str) -> basalt_core::CoreResult<()> {
        panic!("不该写钥匙串:{key}");
    }
    fn delete(&self, key: &str) -> basalt_core::CoreResult<()> {
        panic!("不该删钥匙串:{key}");
    }
    fn delete_quietly(&self, _key: &str) -> basalt_core::CoreResult<QuietDelete> {
        Ok(QuietDelete::Removed)
    }
}

#[tokio::test]
async fn connections_without_a_saved_password_never_read_the_keychain() {
    let h = harness().await;
    let state = AppState::new(
        h._dir.path().join("quiet/connections.json"),
        Arc::new(UnreadableKeychain),
    );
    let saved = service::save_connection(&state, sqlite_draft(&h.sqlite_path))
        .await
        .unwrap();
    let renamed = ConnectionDraft {
        id: Some(saved.id.clone()),
        name: "改个名".into(),
        ..sqlite_draft(&h.sqlite_path)
    };
    service::save_connection(&state, renamed).await.unwrap();
    service::open_connection(&state, &saved.id, None, None)
        .await
        .unwrap();
    let draft = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..sqlite_draft(&h.sqlite_path)
    };
    service::test_connection(&state, draft).await.unwrap();
    let copy = service::duplicate_connection(&state, &saved.id)
        .await
        .unwrap();
    service::delete_connection(&state, &copy.id).await.unwrap();
}

struct ForeignKeychain;

impl SecretStore for ForeignKeychain {
    fn get(&self, _key: &str) -> basalt_core::CoreResult<Option<String>> {
        Ok(None)
    }
    fn set(&self, _key: &str, _secret: &str) -> basalt_core::CoreResult<()> {
        Ok(())
    }
    fn delete(&self, _key: &str) -> basalt_core::CoreResult<()> {
        panic!("删除连接不该用会弹授权框的删除(keyring 删除前会先读出密码)");
    }
    fn delete_quietly(&self, _key: &str) -> basalt_core::CoreResult<QuietDelete> {
        Ok(QuietDelete::NeedsAuthorization)
    }
}

#[tokio::test]
async fn deleting_never_prompts_and_reports_a_password_it_could_not_remove() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        dir.path().join("connections.json"),
        Arc::new(ForeignKeychain),
    );
    let saved = service::save_connection(&state, pg_draft(Some("pw"), true))
        .await
        .unwrap();

    let deleted = service::delete_connection(&state, &saved.id).await.unwrap();
    assert!(deleted.password_left);
    assert!(service::list_connections(&state).await.unwrap().is_empty());
}

// 用只读目录造写入失败:Windows 的只读属性挡不住在目录里建文件,只在 Unix 上跑。
#[cfg(unix)]
#[tokio::test]
async fn config_write_failure_rolls_back_the_new_password() {
    use std::os::unix::fs::PermissionsExt;

    let h = harness().await;
    let saved = service::save_connection(&h.state, pg_draft(Some("old"), true))
        .await
        .unwrap();
    let config_dir = h.state.config_dir().to_path_buf();
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let edit = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..pg_draft(Some("new"), true)
    };
    let result = service::save_connection(&h.state, edit).await;
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(result.unwrap_err().code, "storage");
    assert_eq!(
        h.secrets.get(&password_key(&saved.id)).unwrap().as_deref(),
        Some("old"),
        "配置没写进去,密码也应该回到原值"
    );
}

#[tokio::test]
async fn postgres_uses_saved_password_for_open_and_test() {
    if !pg_enabled() {
        return;
    }
    let h = harness().await;
    let saved = service::save_connection(&h.state, pg_draft(Some(TEST_PASSWORD), true))
        .await
        .unwrap();

    // 表单带 id、没填密码:测试连接要用钥匙串里的密码。
    let draft = ConnectionDraft {
        id: Some(saved.id.clone()),
        ..pg_draft(None, true)
    };
    let server = service::test_connection(&h.state, draft).await.unwrap();
    assert_eq!(server.product, "PostgreSQL");

    service::open_connection(&h.state, &saved.id, None, None)
        .await
        .unwrap();
    let schemas = service::list_schemas(&h.state, &saved.id, "postgres")
        .await
        .unwrap();
    assert!(schemas.contains(&"public".to_owned()), "{schemas:?}");
    service::close_connection(&h.state, &saved.id).await;
}

#[tokio::test]
async fn postgres_prompted_password_opens_without_saving() {
    if !pg_enabled() {
        return;
    }
    let h = harness().await;
    let saved = service::save_connection(&h.state, pg_draft(None, false))
        .await
        .unwrap();

    let error = service::open_connection(&h.state, &saved.id, Some("wrong".into()), None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "authentication");

    service::open_connection(&h.state, &saved.id, Some(TEST_PASSWORD.into()), None)
        .await
        .unwrap();
    assert_eq!(
        h.secrets.get(&password_key(&saved.id)).unwrap(),
        None,
        "临时输入的密码不应落盘"
    );
    service::close_connection(&h.state, &saved.id).await;
}
