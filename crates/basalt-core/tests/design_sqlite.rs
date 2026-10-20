//! SQLite 的表结构设计:能直接 ALTER 的直接做,其余重建表;真实文件、无需容器。

mod common;
mod design_support;

use basalt_core::design::{ExecMode, FkAction, TableDesign};
use basalt_core::session::Session;
use basalt_core::table::TableRef;
use common::*;
use design_support::*;

const SCRIPT: &str = "CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT UNIQUE);
                      INSERT INTO tags VALUES (1, 'a');";

fn table(name: &str) -> TableRef {
    TableRef {
        database: "main".into(),
        schema: None,
        table: name.into(),
    }
}

fn notes() -> TableDesign {
    let mut id = new_column("id", "INTEGER", true);
    id.auto_increment = true;
    let mut title = new_column("title", "TEXT", false);
    title.default = Some("''".into());
    title.collation = Some("NOCASE".into());
    let mut design = TableDesign {
        table: table("notes"),
        primary_key: vec!["new:id".into()],
        primary_key_name: None,
        columns: vec![
            id,
            title,
            new_column("body", "TEXT", true),
            new_column("tag_id", "INTEGER", true),
        ],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
        triggers: Vec::new(),
        options: Default::default(),
    };
    let mut tag = new_foreign_key(&design, "fk_tag", "tag_id", "tags", "id");
    tag.on_delete = FkAction::Cascade;
    design.foreign_keys.push(tag);
    design
        .indexes
        .push(new_index(&design, "idx_title", &[("title", false)], false));
    design
}

async fn sql(pool: &sqlx::SqlitePool, script: &str) {
    sqlx::raw_sql(script).execute(pool).await.unwrap();
}

#[tokio::test]
async fn sqlite_alters_directly_or_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "m5.db", SCRIPT).await;
    let session = Session::open(&sqlite_params(&path)).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{path}"))
        .await
        .unwrap();

    save(&session, None, &notes()).await.unwrap();
    let read = session.read_design(&table("notes")).await.unwrap();
    assert_eq!(column_names(&read), ["id", "title", "body", "tag_id"]);
    assert!(read.columns[0].auto_increment);
    assert_eq!(read.columns[1].collation.as_deref(), Some("NOCASE"));
    assert_eq!(read.columns[1].default.as_deref(), Some("''"));
    assert_eq!(read.foreign_keys[0].name, "fk_tag");
    assert_eq!(read.foreign_keys[0].on_delete, FkAction::Cascade);
    assert_eq!(index_names(&read), ["idx_title"]);
    let unchanged = session
        .preview_design(Some(&table("notes")), &read)
        .await
        .unwrap();
    assert!(
        unchanged.statements.is_empty(),
        "{:?}",
        unchanged.statements
    );

    // 留一个自增空洞:最大 id 是 1,序列已经到 2。
    sql(
        &pool,
        "INSERT INTO notes (title, body, tag_id) VALUES ('t1', 'b1', 1), ('t2', 'b2', 1);
         DELETE FROM notes WHERE id = 2;
         CREATE TABLE log (note_id INTEGER);
         CREATE TRIGGER trg_notes AFTER INSERT ON notes BEGIN INSERT INTO log VALUES (NEW.id); END;
         CREATE VIEW v_notes AS SELECT id, title FROM notes;
         CREATE TABLE attachments (note_id INTEGER REFERENCES notes (id));
         INSERT INTO attachments VALUES (1);",
    )
    .await;

    // 字段改名、加字段、加索引:直接 ALTER。
    let mut next = session.read_design(&table("notes")).await.unwrap();
    column_mut(&mut next, "body").name = "content".into();
    let mut pinned = new_column("pinned", "INTEGER", false);
    pinned.default = Some("0".into());
    next.columns.push(pinned);
    next.indexes
        .push(new_index(&next, "idx_content", &[("content", true)], false));
    let preview = session
        .preview_design(Some(&table("notes")), &next)
        .await
        .unwrap();
    assert_eq!(preview.mode, ExecMode::Transaction);
    save(&session, Some(&table("notes")), &next).await.unwrap();
    let read = session.read_design(&table("notes")).await.unwrap();
    assert_eq!(
        column_names(&read),
        ["id", "title", "content", "tag_id", "pinned"]
    );
    assert_eq!(index_names(&read), ["idx_content", "idx_title"]);

    // 改类型、改非空:重建表。数据、自增序列、索引、触发器、视图、别人的外键都要还在。
    let mut next = read.clone();
    column_mut(&mut next, "title").data_type = "VARCHAR(200)".into();
    column_mut(&mut next, "content").nullable = false;
    let preview = session
        .preview_design(Some(&table("notes")), &next)
        .await
        .unwrap();
    assert_eq!(preview.mode, ExecMode::Rebuild);
    save(&session, Some(&table("notes")), &next).await.unwrap();
    let rebuilt = session.read_design(&table("notes")).await.unwrap();
    assert_eq!(rebuilt.columns[1].data_type, "VARCHAR(200)");
    assert!(!rebuilt.columns[2].nullable);
    assert_eq!(index_names(&rebuilt), ["idx_content", "idx_title"]);
    assert_eq!(rebuilt.triggers.len(), 1);
    assert_eq!(rebuilt.foreign_keys[0].name, "fk_tag");
    sql(
        &pool,
        "INSERT INTO notes (title, content) VALUES ('t3', 'b3')",
    )
    .await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM v_notes ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids, [1, 3], "序列不能退回去重用 2");
    let logged: i64 = sqlx::query_scalar("SELECT count(*) FROM log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(logged, 1, "触发器重建后照常工作");
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(violations.is_empty());

    // 加一个现有数据违反的外键:外键检查不通过,整体回滚。
    let mut bad = rebuilt.clone();
    bad.foreign_keys
        .push(new_foreign_key(&bad, "fk_name", "content", "tags", "name"));
    let error = save(&session, Some(&table("notes")), &bad)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("外键检查没通过"), "{error}");
    let after = session.read_design(&table("notes")).await.unwrap();
    assert_eq!(after.foreign_keys.len(), 1);

    // 重建时视图还引用着要改名的字段:拒绝,让用户分两次做。
    let mut renamed = rebuilt.clone();
    column_mut(&mut renamed, "title").name = "heading".into();
    column_mut(&mut renamed, "heading").data_type = "TEXT".into();
    let error = session
        .preview_design(Some(&table("notes")), &renamed)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("v_notes"), "{error}");

    session.close().await;
    pool.close().await;
}
