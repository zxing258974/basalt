//! 导入导出单测共用:临时 SQLite 文件上开一个连接。

use crate::config::DriverKind;
use crate::driver::ConnectParams;
use crate::session::SessionManager;
use crate::table::TableRef;

pub const SQLITE_SCHEMA: &str = "
    CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, big INTEGER, ratio REAL, avatar BLOB);
    INSERT INTO people VALUES (1, '张三', NULL, 9007199254740993, 0.1, X'00ff');
    INSERT INTO people VALUES (2, 'O''Brien, Jr.', '', -5, NULL, NULL);
    INSERT INTO people VALUES (3, '李四', '两' || char(10) || '行', 0, 1e300, X'');
    CREATE TABLE people_copy (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, big INTEGER, ratio REAL, avatar BLOB);
";

/// 在临时目录里建库、执行脚本,打开成连接 `c1`。
pub async fn open_sqlite(script: &str) -> (tempfile::TempDir, SessionManager) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(script))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let params = ConnectParams {
        driver: DriverKind::Sqlite,
        host: String::new(),
        port: 0,
        username: String::new(),
        password: None,
        database: None,
        file_path: Some(path.to_string_lossy().into_owned()),
        ssl: crate::config::SslSettings::default(),
        ssh: None,
    };
    let sessions = SessionManager::default();
    sessions.open("c1", &params).await.unwrap();
    (dir, sessions)
}

pub fn table_ref(table: &str) -> TableRef {
    TableRef {
        database: "main".into(),
        schema: None,
        table: table.into(),
    }
}

pub fn read_text(path: &str) -> String {
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}
