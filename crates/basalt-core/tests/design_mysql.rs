//! MySQL / MariaDB 的表结构设计:建表、改字段、改名、加索引、加外键,以及中途失败的报告。
//! 需要 BASALT_IT 选中目标,见 common。

mod common;
mod design_support;

use basalt_core::design::{ExecMode, FkAction, TableDesign, TableOptions};
use basalt_core::session::Session;
use basalt_core::table::TableRef;
use common::*;
use design_support::*;

const DB: &str = "basalt_m5";

const FIXTURE: &str = "
    CREATE DATABASE IF NOT EXISTS basalt_m5 CHARACTER SET utf8mb4;
    DROP TABLE IF EXISTS basalt_m5.people, basalt_m5.members, basalt_m5.teams;
    CREATE TABLE basalt_m5.teams (id INT PRIMARY KEY, name VARCHAR(20)) ENGINE = InnoDB;
    INSERT INTO basalt_m5.teams VALUES (1, 'a'), (2, 'b');
";

fn table(name: &str) -> TableRef {
    TableRef {
        database: DB.into(),
        schema: None,
        table: name.into(),
    }
}

async fn setup(target: Target) -> Option<(Session, sqlx::MySqlPool)> {
    if !selected(target) {
        return None;
    }
    let pool = sqlx::MySqlPool::connect(&mysql_url(target))
        .await
        .expect("连接 MySQL 测试容器");
    sqlx::raw_sql(FIXTURE)
        .execute(&pool)
        .await
        .expect("建 M5 测试库");
    let session = Session::open(&params(target, TEST_PASSWORD, None))
        .await
        .unwrap();
    Some((session, pool))
}

fn members() -> TableDesign {
    let mut id = new_column("id", "int", false);
    id.auto_increment = true;
    let mut name = new_column("name", "varchar(50)", false);
    name.default = Some("''".into());
    name.comment = Some("姓名".into());
    let mut created = new_column("created_at", "timestamp", true);
    created.default = Some("CURRENT_TIMESTAMP".into());
    TableDesign {
        version: String::new(),
        table: table("members"),
        primary_key: vec!["new:id".into()],
        primary_key_name: None,
        columns: vec![
            id,
            name,
            new_column("team_id", "int", true),
            new_column("note", "text", true),
            created,
        ],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
        triggers: Vec::new(),
        options: TableOptions {
            engine: Some("InnoDB".into()),
            comment: Some("成员".into()),
            ..TableOptions::default()
        },
    }
}

async fn scenario(target: Target) {
    let Some((session, pool)) = setup(target).await else {
        return;
    };
    let mysql57 = target.name == "mysql57";

    // 建表
    let preview = session.preview_design(None, &members()).await.unwrap();
    assert_eq!(preview.mode, ExecMode::Autocommit);
    assert!(preview.statements[0].starts_with("CREATE TABLE `basalt_m5`.`members`"));
    save(&session, None, &members()).await.unwrap();

    let read = session.read_design(&table("members")).await.unwrap();
    assert_eq!(
        column_names(&read),
        ["id", "name", "team_id", "note", "created_at"]
    );
    assert!(read.columns[0].auto_increment);
    assert!(!read.columns[1].nullable);
    assert_eq!(read.columns[1].default.as_deref(), Some("''"));
    assert_eq!(read.columns[1].comment.as_deref(), Some("姓名"));
    let created = read.columns[4].default.clone().unwrap_or_default();
    assert!(
        created.to_lowercase().starts_with("current_timestamp"),
        "{created}"
    );
    assert_eq!(read.primary_key, [id_of(&read, "id")]);
    assert_eq!(read.options.engine.as_deref(), Some("InnoDB"));
    assert_eq!(read.options.comment.as_deref(), Some("成员"));
    // 读出来的结构原样提交,不该有任何语句。
    let unchanged = session
        .preview_design(Some(&table("members")), &read)
        .await
        .unwrap();
    assert!(
        unchanged.statements.is_empty(),
        "{:?}",
        unchanged.statements
    );

    sqlx::raw_sql("INSERT INTO basalt_m5.members (name, team_id, note) VALUES ('Ann', 1, 'x')")
        .execute(&pool)
        .await
        .unwrap();

    // 一次保存:字段改名并改类型、加字段、加索引、加外键、改注释、表改名。
    let mut next = read.clone();
    column_mut(&mut next, "name").name = "full_name".into();
    column_mut(&mut next, "full_name").data_type = "varchar(80)".into();
    next.columns.insert(2, new_column("age", "int", true));
    next.indexes
        .push(new_index(&next, "idx_age", &[("age", true)], false));
    next.indexes
        .push(new_index(&next, "uk_full", &[("full_name", false)], true));
    let mut team = new_foreign_key(&next, "fk_team", "team_id", "teams", "id");
    team.on_delete = FkAction::SetNull;
    next.foreign_keys.push(team);
    next.options.comment = Some("人员".into());
    next.table = table("people");
    let outcome = save(&session, Some(&table("members")), &next)
        .await
        .unwrap();
    assert_eq!(outcome.table, table("people"));

    let people = session.read_design(&table("people")).await.unwrap();
    assert_eq!(
        column_names(&people),
        ["id", "full_name", "age", "team_id", "note", "created_at"]
    );
    assert_eq!(people.columns[1].data_type, "varchar(80)");
    assert_eq!(people.columns[1].comment.as_deref(), Some("姓名"));
    let age = people
        .indexes
        .iter()
        .find(|i| i.name == "idx_age")
        .expect("idx_age");
    assert!(mysql57 || age.columns[0].descending, "8.0 起支持降序索引");
    assert!(
        people
            .indexes
            .iter()
            .any(|i| i.name == "uk_full" && i.unique)
    );
    assert_eq!(people.foreign_keys.len(), 1);
    assert_eq!(people.foreign_keys[0].on_delete, FkAction::SetNull);
    assert_eq!(people.foreign_keys[0].ref_columns, ["id"]);
    assert_eq!(people.options.comment.as_deref(), Some("人员"));
    let kept: String = sqlx::query_scalar("SELECT full_name FROM basalt_m5.people")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, "Ann", "改名不能丢数据");

    // 挪字段、删索引、删字段、删外键。
    let mut next = people.clone();
    let created = next.columns.remove(5);
    next.columns.insert(1, created);
    next.columns.retain(|c| c.name != "age");
    next.indexes.retain(|i| i.name != "idx_age");
    next.foreign_keys.clear();
    save(&session, Some(&table("people")), &next).await.unwrap();
    let people = session.read_design(&table("people")).await.unwrap();
    assert_eq!(
        column_names(&people),
        ["id", "created_at", "full_name", "team_id", "note"]
    );
    assert!(!index_names(&people).contains(&"idx_age"));
    assert!(people.foreign_keys.is_empty());

    partial_failure(&session, people).await;
    move_foreign_key(&session).await;
    session.close().await;
    pool.close().await;
}

/// 第 1 条(删外键)生效、第 2 条失败:要如实说哪条生效了,不能说回滚。
async fn partial_failure(session: &Session, people: TableDesign) {
    let mut with_fk = people.clone();
    with_fk.foreign_keys.push(new_foreign_key(
        &with_fk, "fk_team", "team_id", "teams", "id",
    ));
    save(session, Some(&table("people")), &with_fk)
        .await
        .unwrap();
    let with_fk = session.read_design(&table("people")).await.unwrap();

    let mut broken = with_fk.clone();
    broken.foreign_keys.clear();
    let mut bad = new_column("bad", "int", true);
    bad.default = Some("'abc'".into());
    broken.columns.push(bad);
    let error = save(session, Some(&table("people")), &broken)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "partially_applied", "{error}");
    let message = error.to_string();
    assert!(message.contains("已经生效：第 1 条"), "{message}");
    assert!(!message.contains("回滚了"), "{message}");

    let after = session.read_design(&table("people")).await.unwrap();
    assert!(after.foreign_keys.is_empty(), "第 1 条已经生效");
    assert!(!column_names(&after).contains(&"bad"));

    // 草稿是按第 1 条生效之前的结构做的:指纹对不上,拒绝。
    let error = session
        .preview_design(Some(&table("people")), &broken)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("被改过"), "{error}");

    // 预览之后结构变了:拒绝执行。
    let error = session
        .apply_design(
            Some(&table("people")),
            &after,
            Some(&["ALTER TABLE x".into()]),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("预览之后变了"));

    // 第 1 条就失败:没有任何改动生效。
    let error = save(
        session,
        None,
        &TableDesign {
            version: String::new(),
            table: table("teams"),
            ..members()
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("没有任何改动生效"), "{error}");
}

/// 外键换到另一个字段、名字不变:MySQL 当初自动建的同名索引要先删掉,否则 Duplicate key name。
async fn move_foreign_key(session: &Session) {
    let mut next = session.read_design(&table("people")).await.unwrap();
    next.columns.push(new_column("owner_team", "int", true));
    next.foreign_keys
        .push(new_foreign_key(&next, "fk_team", "team_id", "teams", "id"));
    save(session, Some(&table("people")), &next).await.unwrap();

    let mut moved = session.read_design(&table("people")).await.unwrap();
    assert!(
        index_names(&moved).contains(&"fk_team"),
        "MySQL 自动建了同名索引"
    );
    moved.foreign_keys[0].columns = vec![id_of(&moved, "owner_team")];
    save(session, Some(&table("people")), &moved).await.unwrap();
    let after = session.read_design(&table("people")).await.unwrap();
    assert_eq!(after.foreign_keys[0].columns, [id_of(&after, "owner_team")]);
}

#[tokio::test]
async fn mysql57_designs_tables() {
    scenario(MYSQL57).await;
}

#[tokio::test]
async fn mysql84_designs_tables() {
    scenario(MYSQL84).await;
}

#[tokio::test]
async fn mariadb_designs_tables() {
    scenario(MARIADB).await;
}
