//! Redis 的服务器监控:INFO 计数器、CLIENT LIST 里认出自己和阻塞的客户端、
//! 解除阻塞、断开别的客户端、不能断开自己。需要 BASALT_IT 选中 redis(见 common)。不写键。

mod common;

use std::sync::Arc;
use std::time::Duration;

use basalt_core::driver::ConnectParams;
use basalt_core::error::CoreError;
use basalt_core::monitor::{Activity, KillMode, KillTarget};
use basalt_core::redis_db::RedisSession;
use basalt_core::redis_db::console::{ConsoleOutcome, Reply};
use common::*;

async fn open() -> Arc<RedisSession> {
    Arc::new(
        RedisSession::open(&ConnectParams {
            database: Some("3".into()),
            ..params(REDIS, TEST_PASSWORD, None)
        })
        .await
        .unwrap(),
    )
}

fn target(id: i64, mode: KillMode) -> KillTarget {
    KillTarget {
        id,
        mode,
        session_start: None,
        query: None,
    }
}

#[tokio::test]
async fn shows_clients_and_unblocks_or_disconnects_them() {
    if !selected(REDIS) {
        return;
    }
    let session = open().await;
    // 命令行里一条一直在等的 BLPOP。
    let ConsoleOutcome::Ran {
        reply: Reply::Int { value },
        ..
    } = session
        .run_console("m", 3, "CLIENT ID", false)
        .await
        .unwrap()
    else {
        panic!("CLIENT ID 应该回整数");
    };
    let waiting_id: i64 = value.parse().unwrap();
    let waiting = {
        let session = session.clone();
        tokio::spawn(async move {
            session
                .run_console("m", 3, "BLPOP monitor:nothing 0", false)
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    let snapshot = session.server_snapshot().await.unwrap();
    assert!(snapshot.counters.connections >= 2);
    assert!(snapshot.counters.running >= 1, "至少有那条 BLPOP 在等");
    assert!(snapshot.counters.queries.is_some());
    assert!(snapshot.counters.max_connections.is_some());
    let redis = snapshot.redis.expect("Redis 的内存和键数");
    assert!(redis.memory_used.is_some());
    assert!(redis.slowlog.is_some());
    let own = snapshot
        .processes
        .iter()
        .find(|p| p.is_self)
        .expect("列表里认得出自己");
    assert_eq!(own.query.as_deref(), Some("CLIENT LIST"));
    assert!(
        own.detail
            .as_deref()
            .unwrap_or_default()
            .starts_with("basalt")
    );
    let blocked = snapshot
        .processes
        .iter()
        .find(|p| p.id == waiting_id)
        .expect("命令行的连接在列表里");
    assert_eq!(blocked.activity, Activity::Running);
    assert_eq!(blocked.query.as_deref(), Some("BLPOP"));
    assert_eq!(blocked.database.as_deref(), Some("db3"));

    // 解除阻塞:BLPOP 马上回 nil,连接还在。
    session
        .kill_client(&target(waiting_id, KillMode::Query))
        .await
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("解除阻塞后应该马上返回")
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            outcome,
            ConsoleOutcome::Ran {
                reply: Reply::Nil,
                ..
            }
        ),
        "{outcome:?}"
    );
    let error = session
        .kill_client(&target(waiting_id, KillMode::Query))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("没有在等"), "{error}");

    // 断开连接;再断一次就是「已经结束了」。
    session
        .kill_client(&target(waiting_id, KillMode::Connection))
        .await
        .unwrap();
    let error = session
        .kill_client(&target(waiting_id, KillMode::Connection))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("已经结束"), "{error}");

    let error = session
        .kill_client(&target(own.id, KillMode::Connection))
        .await
        .unwrap_err();
    assert!(matches!(error, CoreError::InvalidData(_)), "{error:?}");
    session.close().await;
}

#[tokio::test]
async fn an_account_without_client_rights_still_sees_the_rest() {
    if !selected(REDIS) {
        return;
    }
    let admin = open().await;
    let user = "basalt-monitor-noclient";
    let setup = format!("ACL SETUSER {user} reset on >pw ~* +@all -client");
    admin.run_console("acl", 3, &setup, true).await.unwrap();

    let limited = RedisSession::open(&ConnectParams {
        username: user.into(),
        password: Some("pw".into()),
        database: Some("3".into()),
        ..params(REDIS, TEST_PASSWORD, None)
    })
    .await
    .unwrap();
    let snapshot = limited.server_snapshot().await;
    limited.close().await;
    admin
        .run_console("acl", 3, &format!("ACL DELUSER {user}"), true)
        .await
        .unwrap();

    let snapshot = snapshot.unwrap();
    assert!(snapshot.processes.is_empty());
    assert_eq!(
        snapshot.limited.as_deref(),
        Some("这个账号不能执行 CLIENT LIST，看不到客户端列表。")
    );
    assert!(snapshot.counters.queries.is_some(), "INFO 照样读得到");
    admin.close().await;
}

#[tokio::test]
async fn does_nothing_when_the_server_restarted_since_the_confirmation() {
    if !selected(REDIS) {
        return;
    }
    let session = open().await;
    let snapshot = session.server_snapshot().await.unwrap();
    let other = snapshot
        .processes
        .iter()
        .find(|p| !p.is_self)
        .or(snapshot.processes.first())
        .unwrap();
    assert_eq!(other.session_start, snapshot.redis.unwrap().run_id);
    let error = session
        .kill_client(&KillTarget {
            id: other.id,
            mode: KillMode::Connection,
            session_start: Some("0000000000000000000000000000000000000000".into()),
            query: None,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("重启过"), "{error}");
    session.close().await;
}
