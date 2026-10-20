//! PostgreSQL 的表结构设计:建表、改字段、改名、加索引、加外键,失败整体回滚。
//! 需要 BASALT_IT 选中 postgres,见 common。

mod common;
mod design_support;

use basalt_core::design::{ExecMode, FkAction, TableDesign, TableOptions};
use basalt_core::session::Session;
use basalt_core::table::TableRef;
use common::*;
use design_support::*;

const DB: &str = "basalt_m5";

const FIXTURE: &str = "
    DROP TABLE IF EXISTS public.people, public.members, public.teams CASCADE;
    CREATE TABLE public.teams (id integer PRIMARY KEY, name text);
    INSERT INTO public.teams VALUES (1, 'a'), (2, 'b');
";

fn table(name: &str) -> TableRef {
    TableRef {
        database: DB.into(),
        schema: Some("public".into()),
        table: name.into(),
    }
}

async fn setup() -> Option<(Session, sqlx::PgPool)> {
    if !selected(POSTGRES) {
        return None;
    }
    let admin = sqlx::PgPool::connect(&postgres_url("postgres"))
        .await
        .expect("连接 PG 测试容器");
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
        .bind(DB)
        .fetch_optional(&admin)
        .await
        .unwrap();
    if exists.is_none() {
        sqlx::raw_sql("CREATE DATABASE basalt_m5")
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
    let pool = sqlx::PgPool::connect(&postgres_url(DB)).await.unwrap();
    sqlx::raw_sql(FIXTURE)
        .execute(&pool)
        .await
        .expect("建 M5 测试数据");
    let session = Session::open(&params(POSTGRES, TEST_PASSWORD, None))
        .await
        .unwrap();
    Some((session, pool))
}

fn members() -> TableDesign {
    let mut id = new_column("id", "integer", false);
    id.auto_increment = true;
    let mut name = new_column("name", "varchar(50)", false);
    name.default = Some("''".into());
    name.comment = Some("姓名".into());
    let design = TableDesign {
        table: table("members"),
        primary_key: vec!["new:id".into()],
        primary_key_name: None,
        columns: vec![
            id,
            name,
            new_column("code", "text", true),
            new_column("team_id", "integer", true),
        ],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
        triggers: Vec::new(),
        options: TableOptions {
            comment: Some("成员".into()),
            ..TableOptions::default()
        },
    };
    let index = new_index(&design, "idx_code", &[("code", false)], false);
    TableDesign {
        indexes: vec![index],
        ..design
    }
}

#[tokio::test]
async fn postgres_designs_tables() {
    let Some((session, pool)) = setup().await else {
        return;
    };

    let preview = session.preview_design(None, &members()).await.unwrap();
    assert_eq!(preview.mode, ExecMode::Transaction);
    assert_eq!(preview.statements.len(), 4, "{:?}", preview.statements);
    save(&session, None, &members()).await.unwrap();

    let read = session.read_design(&table("members")).await.unwrap();
    assert_eq!(column_names(&read), ["id", "name", "code", "team_id"]);
    assert!(read.columns[0].auto_increment);
    assert_eq!(read.columns[1].data_type, "character varying(50)");
    assert_eq!(
        read.columns[1].default.as_deref(),
        Some("''::character varying")
    );
    assert_eq!(read.columns[1].comment.as_deref(), Some("姓名"));
    assert_eq!(read.primary_key_name.as_deref(), Some("members_pkey"));
    assert_eq!(index_names(&read), ["idx_code"]);
    assert_eq!(read.options.comment.as_deref(), Some("成员"));
    let unchanged = session
        .preview_design(Some(&table("members")), &read)
        .await
        .unwrap();
    assert!(
        unchanged.statements.is_empty(),
        "{:?}",
        unchanged.statements
    );

    sqlx::raw_sql("INSERT INTO public.members (name, code, team_id) VALUES ('Ann', '42', 1)")
        .execute(&pool)
        .await
        .unwrap();

    let mut next = read.clone();
    column_mut(&mut next, "name").name = "full_name".into();
    column_mut(&mut next, "code").data_type = "integer".into();
    next.columns.push(new_column("age", "integer", true));
    next.indexes
        .push(new_index(&next, "idx_age", &[("age", true)], false));
    let mut team = new_foreign_key(&next, "fk_team", "team_id", "teams", "id");
    team.on_delete = FkAction::Cascade;
    next.foreign_keys.push(team);
    next.options.comment = Some("人员".into());
    next.table = table("people");
    save(&session, Some(&table("members")), &next)
        .await
        .unwrap();

    let people = session.read_design(&table("people")).await.unwrap();
    assert_eq!(
        column_names(&people),
        ["id", "full_name", "code", "team_id", "age"]
    );
    assert_eq!(people.columns[2].data_type, "integer");
    let age = people
        .indexes
        .iter()
        .find(|i| i.name == "idx_age")
        .expect("idx_age");
    assert!(age.columns[0].descending);
    assert_eq!(people.foreign_keys[0].on_delete, FkAction::Cascade);
    assert_eq!(people.options.comment.as_deref(), Some("人员"));
    let (name, code): (String, i32) = sqlx::query_as("SELECT full_name, code FROM public.people")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((name.as_str(), code), ("Ann", 42), "改名、改类型不能丢数据");

    // 第 1 条(改名)成功、第 2 条失败:整体回滚,改名也没有生效。
    let mut broken = people.clone();
    column_mut(&mut broken, "full_name").name = "nick".into();
    let mut bad = new_column("bad", "integer", true);
    bad.default = Some("'abc'".into());
    broken.columns.push(bad);
    let error = save(&session, Some(&table("people")), &broken)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("所有修改都已回滚"), "{error}");
    let after = session.read_design(&table("people")).await.unwrap();
    assert!(column_names(&after).contains(&"full_name"));

    let mut reordered = people.clone();
    reordered.columns.swap(1, 2);
    let error = session
        .preview_design(Some(&table("people")), &reordered)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("顺序"));

    // 删外键、删索引、删字段、改回可空。
    let mut next = people.clone();
    next.foreign_keys.clear();
    next.indexes.retain(|i| i.name != "idx_age");
    next.columns.retain(|c| c.name != "age");
    column_mut(&mut next, "full_name").nullable = true;
    column_mut(&mut next, "full_name").default = None;
    save(&session, Some(&table("people")), &next).await.unwrap();
    let people = session.read_design(&table("people")).await.unwrap();
    assert!(people.foreign_keys.is_empty());
    assert_eq!(index_names(&people), ["idx_code"]);
    assert!(people.columns[1].nullable && people.columns[1].default.is_none());

    session.close().await;
    pool.close().await;
}
