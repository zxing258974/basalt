//! 服务器监控:会话列表、计数器、终止会话,以及权限不够时看到什么。需要 BASALT_IT 选中目标,见 common。

mod common;

use std::time::{Duration, Instant};

use basalt_core::monitor::{Activity, KillMode, KillTarget, ServerProcess};
use basalt_core::session::Session;
use common::*;
use sqlx::Executor;

/// 等快照里出现满足条件的会话(另一条连接上的语句刚发出去,服务器可能还没登记)。
async fn wait_for_process(
    session: &Session,
    found: impl Fn(&ServerProcess) -> bool,
) -> ServerProcess {
    for _ in 0..50 {
        let snapshot = session.server_snapshot().await.unwrap();
        if let Some(process) = snapshot.processes.into_iter().find(|p| found(p)) {
            return process;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("快照里一直没出现要找的会话");
}

/// 照快照里看到的样子终止,和界面上确认之后发出去的一样。
fn target(process: &ServerProcess, mode: KillMode) -> KillTarget {
    KillTarget {
        id: process.id,
        mode,
        session_start: process.session_start.clone(),
        query: process.query.clone(),
    }
}

fn by_id(id: i64, mode: KillMode) -> KillTarget {
    KillTarget {
        id,
        mode,
        session_start: None,
        query: None,
    }
}

#[tokio::test]
async fn mysql_family_lists_sessions_and_kills_a_long_query() {
    for target_db in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let name = target_db.name;
        let session = Session::open(&params(target_db, TEST_PASSWORD, None))
            .await
            .unwrap();
        let snapshot = session.server_snapshot().await.unwrap();
        let own: Vec<_> = snapshot.processes.iter().filter(|p| p.is_self).collect();
        assert_eq!(own.len(), 1, "{name}");
        let own_id = own[0].id;
        assert!(snapshot.counters.connections >= 1, "{name}");
        assert!(snapshot.counters.queries.is_some(), "{name}");
        assert!(snapshot.counters.max_connections.is_some(), "{name}");
        assert!(snapshot.counters.cache_reads.is_some(), "{name}");
        assert_eq!(snapshot.limited, None, "root 看得到所有会话：{name}");
        // 取数总在同一条专用连接上。
        let again = session.server_snapshot().await.unwrap();
        let own_again: Vec<_> = again
            .processes
            .iter()
            .filter(|p| p.is_self)
            .map(|p| p.id)
            .collect();
        assert_eq!(own_again, [own_id], "{name}");
        let error = session
            .kill_process(&by_id(own_id, KillMode::Connection))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("取监控数据用的连接"),
            "{name}: {error}"
        );

        // 另一条连接上跑一条长语句,在快照里找到它、只终止这条语句。
        let other = sqlx::MySqlPool::connect(&mysql_url(target_db))
            .await
            .unwrap();
        let mut conn = other.acquire().await.unwrap();
        let started = Instant::now();
        let sleeping = tokio::spawn(async move {
            let _ = (&mut *conn).execute("SELECT SLEEP(20)").await;
            conn
        });
        let process = wait_for_process(&session, |p| {
            p.query.as_deref().is_some_and(|q| q.contains("SLEEP(20)"))
        })
        .await;
        assert_eq!(process.activity, Activity::Running, "{name}");
        assert!(!process.is_self, "{name}");

        // 确认时看到的语句已经不是它在执行的了:不动手。
        let stale = KillTarget {
            query: Some("SELECT SLEEP(1)".into()),
            ..target(&process, KillMode::Query)
        };
        let error = session.kill_process(&stale).await.unwrap_err();
        assert!(
            error.to_string().contains("不在执行刚才那条语句"),
            "{name}: {error}"
        );

        session
            .kill_process(&target(&process, KillMode::Query))
            .await
            .unwrap();
        let mut conn = tokio::time::timeout(Duration::from_secs(10), sleeping)
            .await
            .expect("KILL QUERY 之后语句应该马上结束")
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(15), "{name}");

        // 连接还在;再整个断开它。
        let id: u64 = sqlx::query_scalar("SELECT CAST(CONNECTION_ID() AS UNSIGNED)")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        let id = i64::try_from(id).unwrap();
        let idle = wait_for_process(&session, |p| p.id == id).await;
        assert_eq!(idle.activity, Activity::Idle, "{name}");
        session
            .kill_process(&target(&idle, KillMode::Connection))
            .await
            .unwrap();
        assert!(
            sqlx::query("SELECT 1").execute(&mut *conn).await.is_err(),
            "连接断开后不能再用：{name}"
        );
        drop(conn);
        let error = session
            .kill_process(&by_id(id, KillMode::Connection))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("已经结束"), "{name}: {error}");
        other.close().await;
        session.close().await;
    }
}

#[tokio::test]
async fn mysql_accounts_without_process_only_see_their_own_sessions() {
    for target_db in MYSQL_FAMILY.into_iter().filter(|t| selected(*t)) {
        let name = target_db.name;
        let admin = sqlx::MySqlPool::connect(&mysql_url(target_db))
            .await
            .unwrap();
        admin
            .execute(
                "DROP USER IF EXISTS 'basalt_mon'@'%'; \
                 CREATE USER 'basalt_mon'@'%' IDENTIFIED BY 'basalt-test'",
            )
            .await
            .unwrap();
        let mut limited = params(target_db, TEST_PASSWORD, None);
        limited.username = "basalt_mon".into();
        let session = Session::open(&limited).await.unwrap();
        let snapshot = session.server_snapshot().await.unwrap();
        assert!(snapshot.limited.is_some(), "{name}");
        assert!(
            snapshot
                .processes
                .iter()
                .all(|p| p.user.as_deref() == Some("basalt_mon")),
            "{name}: {:?}",
            snapshot.processes
        );
        session.close().await;
        admin.execute("DROP USER 'basalt_mon'@'%'").await.unwrap();
        admin.close().await;
    }
}

#[tokio::test]
async fn postgres_lists_sessions_and_cancels_or_terminates_them() {
    if !selected(POSTGRES) {
        return;
    }
    let session = Session::open(&params(POSTGRES, TEST_PASSWORD, None))
        .await
        .unwrap();
    let snapshot = session.server_snapshot().await.unwrap();
    assert_eq!(snapshot.processes.iter().filter(|p| p.is_self).count(), 1);
    assert!(snapshot.processes.iter().all(|p| p.session_start.is_some()));
    assert!(snapshot.counters.commits.is_some());
    assert!(snapshot.counters.uptime_seconds.is_some());
    assert_eq!(
        snapshot.counters.queries, None,
        "PostgreSQL 没有全局的查询计数"
    );

    let other = sqlx::PgPool::connect(&postgres_url("postgres"))
        .await
        .unwrap();
    let mut conn = other.acquire().await.unwrap();
    let sleeping = tokio::spawn(async move {
        let result = sqlx::query("SELECT pg_sleep(20)").execute(&mut *conn).await;
        (result.is_err(), conn)
    });
    let process = wait_for_process(&session, |p| {
        p.query
            .as_deref()
            .is_some_and(|q| q.contains("pg_sleep(20)"))
            && p.activity == Activity::Running
    })
    .await;
    // 进程号对上了,连上的时刻对不上:是后来的另一个会话,不动手。
    let reused = KillTarget {
        session_start: Some("2000-01-01 00:00:00+00".into()),
        ..target(&process, KillMode::Query)
    };
    let error = session.kill_process(&reused).await.unwrap_err();
    assert!(error.to_string().contains("已经结束"), "{error}");

    session
        .kill_process(&target(&process, KillMode::Query))
        .await
        .unwrap();
    let (failed, mut conn) = tokio::time::timeout(Duration::from_secs(10), sleeping)
        .await
        .expect("取消之后语句应该马上结束")
        .unwrap();
    assert!(failed, "被取消的语句报错");

    // 开着事务、闲着:算「事务中」,没有语句可终止。
    conn.execute("BEGIN").await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let waiting = wait_for_process(&session, |p| {
        p.id == i64::from(pid) && p.activity == Activity::InTransaction
    })
    .await;
    let error = session
        .kill_process(&target(&waiting, KillMode::Query))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("不在执行刚才那条语句"),
        "{error}"
    );

    session
        .kill_process(&target(&waiting, KillMode::Connection))
        .await
        .unwrap();
    assert!(sqlx::query("SELECT 1").execute(&mut *conn).await.is_err());
    drop(conn);
    let error = session
        .kill_process(&by_id(i64::from(pid), KillMode::Connection))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("已经结束"), "{error}");
    other.close().await;
    session.close().await;
}

#[tokio::test]
async fn postgres_roles_without_stats_see_other_sessions_but_not_what_they_do() {
    if !selected(POSTGRES) {
        return;
    }
    let admin = sqlx::PgPool::connect(&postgres_url("postgres"))
        .await
        .unwrap();
    admin
        .execute(
            "DROP ROLE IF EXISTS basalt_mon; \
             CREATE ROLE basalt_mon LOGIN PASSWORD 'basalt-test'",
        )
        .await
        .unwrap();
    // 管理员的连接占着一个会话,看它在不在列表里。
    let mut busy = admin.acquire().await.unwrap();
    let busy_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *busy)
        .await
        .unwrap();

    let mut limited = params(POSTGRES, TEST_PASSWORD, None);
    limited.username = "basalt_mon".into();
    let session = Session::open(&limited).await.unwrap();
    let snapshot = session.server_snapshot().await.unwrap();
    assert!(snapshot.limited.is_some());
    let hidden = snapshot
        .processes
        .iter()
        .find(|p| p.id == i64::from(busy_pid))
        .expect("别人的会话也列出来");
    assert_eq!(hidden.activity, Activity::Unknown);
    assert_eq!(hidden.query, None, "没权限时给的提示不当语句");
    assert_eq!(hidden.user.as_deref(), Some("postgres"));
    assert!(
        snapshot.processes.iter().all(|p| p.user.is_some()),
        "后台进程不算：{:?}",
        snapshot.processes
    );
    let visible = u64::try_from(snapshot.processes.len()).unwrap();
    assert_eq!(snapshot.counters.connections, visible, "连接数和列表一致");
    session.close().await;

    drop(busy);
    admin.execute("DROP ROLE basalt_mon").await.unwrap();
    admin.close().await;
}

#[tokio::test]
async fn sqlite_has_no_server_to_watch() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "m.db", "CREATE TABLE t (id INTEGER)").await;
    let session = Session::open(&sqlite_params(&path)).await.unwrap();
    let error = session.server_snapshot().await.unwrap_err();
    assert_eq!(error.code(), "unsupported");
    session.close().await;
}
