//! Redis 命令行:每个标签页自己的连接(SELECT、MULTI 只对它生效)、危险命令先确认、
//! 不支持的命令直接拒绝、错误回复照原样、停下正在等的命令、关掉标签页。
//! 需要 BASALT_IT 选中 redis(见 common)。测试并行跑:每个测试清空、使用自己的库(1–5),别的测试不碰。

mod common;

use std::time::Duration;

use basalt_core::driver::ConnectParams;
use basalt_core::error::CoreError;
use basalt_core::redis_db::RedisSession;
use basalt_core::redis_db::console::{ConsoleOutcome, Reply};
use common::*;

/// 打开会话,清空 `dbs` 这几个库。
async fn open(dbs: &[u32]) -> RedisSession {
    let session = RedisSession::open(&ConnectParams {
        database: Some("4".into()),
        ..params(REDIS, TEST_PASSWORD, None)
    })
    .await
    .unwrap();
    // 每个库各用一个命令行:命令行连上之后就在自己的库里,`db` 参数只在连接时用。
    for &db in dbs {
        let console = format!("setup-{db}");
        let outcome = session
            .run_console(&console, db, "FLUSHDB", true)
            .await
            .unwrap();
        assert!(matches!(outcome, ConsoleOutcome::Ran { .. }));
        session.close_console(&console).await;
    }
    session
}

/// 跑一行,要求执行了;返回回复和执行后所在的库、在不在事务里。
async fn run(session: &RedisSession, console: &str, db: u32, line: &str) -> (Reply, u32, bool) {
    match session.run_console(console, db, line, false).await.unwrap() {
        ConsoleOutcome::Ran {
            reply,
            db,
            transaction,
            ..
        } => (reply, db, transaction),
        other => panic!("「{line}」应该执行了：{other:?}"),
    }
}

fn bulk(text: &str) -> Reply {
    Reply::Bulk {
        text: text.into(),
        binary: false,
        bytes: text.len(),
        truncated: false,
    }
}

fn ok() -> Reply {
    Reply::Status { text: "OK".into() }
}

#[tokio::test]
async fn each_console_keeps_its_own_database_and_transaction() {
    if !selected(REDIS) {
        return;
    }
    let session = open(&[4, 5]).await;
    assert_eq!(
        run(&session, "a", 4, "SET greeting \"你好 世界\"").await.0,
        ok()
    );
    assert_eq!(
        run(&session, "a", 4, "get greeting").await.0,
        bulk("你好 世界")
    );

    // SELECT 之后这个标签页在库 5,另一个标签页还在库 4。
    let (reply, db, _) = run(&session, "a", 4, "SELECT 5").await;
    assert_eq!((reply, db), (ok(), 5));
    assert_eq!(run(&session, "a", 5, "GET greeting").await.0, Reply::Nil);
    assert_eq!(
        run(&session, "b", 4, "GET greeting").await.0,
        bulk("你好 世界")
    );

    let (_, _, transaction) = run(&session, "a", 5, "MULTI").await;
    assert!(transaction);
    let (reply, _, transaction) = run(&session, "a", 5, "INCR n").await;
    assert_eq!(
        reply,
        Reply::Status {
            text: "QUEUED".into()
        }
    );
    assert!(transaction);
    // 事务在 a 的连接上,b 看不到还没执行的 INCR。
    assert_eq!(run(&session, "b", 5, "GET n").await.0, Reply::Nil);
    let (reply, _, transaction) = run(&session, "a", 5, "EXEC").await;
    assert_eq!(
        reply,
        Reply::Array {
            items: vec![Reply::Int { value: "1".into() }],
            total: 1
        }
    );
    assert!(!transaction);
    session.close().await;
}

#[tokio::test]
async fn shows_replies_as_redis_sends_them() {
    if !selected(REDIS) {
        return;
    }
    let session = open(&[3]).await;
    run(&session, "c", 3, r#"RPUSH l a "b c" "\x00\xff""#).await;
    assert_eq!(
        run(&session, "c", 3, "LRANGE l 0 -1").await.0,
        Reply::Array {
            items: vec![
                bulk("a"),
                bulk("b c"),
                Reply::Bulk {
                    text: r"\x00\xff".into(),
                    binary: true,
                    bytes: 2,
                    truncated: false,
                },
            ],
            total: 3,
        }
    );
    assert_eq!(
        run(&session, "c", 3, "HGETALL missing").await.0,
        Reply::Array {
            items: vec![],
            total: 0
        }
    );
    // 服务器报的错是一种回复,不是失败。
    let (reply, ..) = run(&session, "c", 3, "LPUSH").await;
    let Reply::Error { text } = reply else {
        panic!("应该是错误回复：{reply:?}");
    };
    assert!(text.starts_with("ERR wrong number of arguments"), "{text}");
    let (reply, ..) = run(&session, "c", 3, "INCR l").await;
    assert!(
        matches!(reply, Reply::Error { ref text } if text.starts_with("WRONGTYPE")),
        "{reply:?}"
    );

    let error = session
        .run_console("c", 3, "SET k \"open", false)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("引号没有配对"), "{error}");
    session.close().await;
}

#[tokio::test]
async fn asks_before_dangerous_commands_and_refuses_subscriptions() {
    if !selected(REDIS) {
        return;
    }
    let session = open(&[2]).await;
    run(&session, "d", 2, "SET keep 1").await;
    let outcome = session.run_console("d", 2, "flushdb", false).await.unwrap();
    let ConsoleOutcome::Confirm { reason } = outcome else {
        panic!("FLUSHDB 应该先确认：{outcome:?}");
    };
    assert!(reason.contains("当前库"), "{reason}");
    assert_eq!(
        run(&session, "d", 2, "EXISTS keep").await.0,
        Reply::Int { value: "1".into() }
    );

    let outcome = session.run_console("d", 2, "FLUSHDB", true).await.unwrap();
    assert!(matches!(outcome, ConsoleOutcome::Ran { .. }));
    assert_eq!(
        run(&session, "d", 2, "EXISTS keep").await.0,
        Reply::Int { value: "0".into() }
    );

    for line in ["SUBSCRIBE news", "monitor", "HELLO 3", "client reply off"] {
        let error = session.run_console("d", 2, line, true).await.unwrap_err();
        assert!(
            matches!(error, CoreError::Unsupported(_)),
            "{line}: {error:?}"
        );
    }
    session.close().await;
}

#[tokio::test]
async fn stopping_a_blocked_command_reconnects_on_the_next_one() {
    if !selected(REDIS) {
        return;
    }
    let session = std::sync::Arc::new(open(&[1]).await);
    run(&session, "e", 4, "SELECT 1").await;
    let waiting = {
        let session = session.clone();
        tokio::spawn(async move { session.run_console("e", 1, "BLPOP nothing 0", false).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiting.is_finished(), "BLPOP 应该一直等着");
    session.cancel_console("e").await;
    let error = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("停下之后应该马上返回")
        .unwrap()
        .unwrap_err();
    assert!(matches!(error, CoreError::Network(_)), "{error:?}");

    // 重新连上时回到界面上显示的库。
    run(&session, "e", 1, "SET back 1").await;
    assert_eq!(run(&session, "f", 1, "GET back").await.0, bulk("1"));

    session.close_console("e").await;
    session.close().await;
}

#[tokio::test]
async fn lists_command_names_for_completion() {
    if !selected(REDIS) {
        return;
    }
    let session = open(&[]).await;
    let names = session.command_names().await.unwrap();
    assert!(names.contains(&"GET".to_owned()));
    assert!(names.contains(&"HSET".to_owned()));
    assert!(names.windows(2).all(|w| w[0] < w[1]), "排好序、不重复");
    session.close().await;
}

#[tokio::test]
async fn a_select_inside_a_transaction_counts_only_once_it_ran() {
    if !selected(REDIS) {
        return;
    }
    let session = open(&[]).await;
    run(&session, "g", 4, "MULTI").await;
    let (reply, db, _) = run(&session, "g", 4, "SELECT 5").await;
    assert_eq!(
        (reply, db),
        (
            Reply::Status {
                text: "QUEUED".into()
            },
            4
        ),
        "排队时还没换库"
    );
    let (_, db, transaction) = run(&session, "g", 4, "DISCARD").await;
    assert_eq!((db, transaction), (4, false), "放弃了就没换");

    run(&session, "g", 4, "MULTI").await;
    run(&session, "g", 4, "SELECT 5").await;
    let (_, db, _) = run(&session, "g", 4, "EXEC").await;
    assert_eq!(db, 5);

    // 事务里换到不存在的库:EXEC 时这一条报错,库号不能跟着记错。
    run(&session, "g", 5, "MULTI").await;
    run(&session, "g", 5, "SELECT 99").await;
    let (reply, db, _) = run(&session, "g", 5, "EXEC").await;
    assert!(
        matches!(&reply, Reply::Array { items, .. } if matches!(items[0], Reply::Error { .. })),
        "{reply:?}"
    );
    assert_eq!(db, 5);
    session.close().await;
}

#[tokio::test]
async fn a_dropped_connection_does_not_quietly_lose_a_transaction() {
    if !selected(REDIS) {
        return;
    }
    let session = open(&[]).await;
    let (Reply::Int { value: id }, ..) = run(&session, "h", 4, "CLIENT ID").await else {
        panic!("CLIENT ID 应该回整数");
    };
    run(&session, "h", 4, "MULTI").await;
    // 像服务器踢掉空闲连接那样,从另一个命令行把它断开。
    let killed = session
        .run_console("killer", 4, &format!("CLIENT KILL ID {id}"), true)
        .await
        .unwrap();
    assert!(
        matches!(killed, ConsoleOutcome::Ran { reply: Reply::Int { ref value }, .. } if value == "1")
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    let error = session
        .run_console("h", 4, "PING", false)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, CoreError::Network(message) if message.contains("事务")),
        "{error:?}"
    );
    // 报过一次,再执行就重新连上,不在事务里。
    let (reply, _, transaction) = run(&session, "h", 4, "PING").await;
    assert_eq!(
        reply,
        Reply::Status {
            text: "PONG".into()
        }
    );
    assert!(!transaction);
    session.close().await;
}

#[tokio::test]
async fn a_closed_console_runs_nothing_more() {
    if !selected(REDIS) {
        return;
    }
    let session = open(&[]).await;
    run(&session, "i", 4, "PING").await;
    session.close().await;
    let error = session
        .run_console("i", 4, "PING", false)
        .await
        .unwrap_err();
    assert!(matches!(error, CoreError::Network(_)), "{error:?}");
}
