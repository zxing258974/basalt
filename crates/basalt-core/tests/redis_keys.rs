//! Redis 键浏览与编辑:扫描、读各种类型的值、核对后提交修改、字符串、新建 / 改名 / 过期 / 删除。
//! 需要 BASALT_IT 选中 redis(见 common)。每个测试用自己的库号和键前缀,互不干扰。

mod common;

use std::collections::BTreeMap;

use basalt_core::driver::ConnectParams;
use basalt_core::redis_db::RedisSession;
use basalt_core::redis_db::edit::NewKey;
use basalt_core::redis_db::keys::{KeyName, KeyValue, ScanRequest};
use basalt_core::table::{EditValues, RowChange, RowKey};
use basalt_core::value::CellValue;
use common::*;
use redis::aio::MultiplexedConnection;

async fn raw(db: u32) -> MultiplexedConnection {
    let url = format!("redis://:{TEST_PASSWORD}@127.0.0.1:{}/{db}", REDIS.port);
    redis::Client::open(url)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .expect("连不上 Redis 测试容器")
}

/// 清空库 `db`,写入测试数据,打开一个以它为默认库的会话。
async fn fresh(db: u32, setup: &[&[&str]]) -> (RedisSession, MultiplexedConnection) {
    let mut conn = raw(db).await;
    let _: () = redis::cmd("FLUSHDB").query_async(&mut conn).await.unwrap();
    for command in setup {
        let mut cmd = redis::cmd(command[0]);
        for arg in &command[1..] {
            cmd.arg(*arg);
        }
        let _: redis::Value = cmd.query_async(&mut conn).await.unwrap();
    }
    let session = RedisSession::open(&ConnectParams {
        database: Some(db.to_string()),
        ..params(REDIS, TEST_PASSWORD, None)
    })
    .await
    .unwrap();
    (session, conn)
}

fn id(name: &str) -> String {
    KeyName::from_bytes(name.as_bytes()).id
}

fn row(pairs: &[(&str, CellValue)]) -> RowKey {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn values(pairs: &[(&str, &str)]) -> EditValues {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), Some((*v).to_owned())))
        .collect()
}

fn text(value: &str) -> CellValue {
    CellValue::Text(value.into())
}

fn table(value: &KeyValue) -> &basalt_core::table::TablePage {
    match value {
        KeyValue::Table { page, .. } => page,
        other => panic!("应该是表格：{other:?}"),
    }
}

#[tokio::test]
async fn scans_by_pattern_and_type_with_ttls() {
    if !selected(REDIS) {
        return;
    }
    let mut setup: Vec<Vec<String>> = (0..120)
        .map(|i| vec!["SET".into(), format!("product:{i}"), "x".into()])
        .collect();
    setup.push(vec![
        "HSET".into(),
        "user:1".into(),
        "name".into(),
        "a".into(),
    ]);
    setup.push(vec![
        "SET".into(),
        "user:1:token".into(),
        "t".into(),
        "EX".into(),
        "300".into(),
    ]);
    let setup: Vec<Vec<&str>> = setup
        .iter()
        .map(|c| c.iter().map(String::as_str).collect())
        .collect();
    let setup: Vec<&[&str]> = setup.iter().map(Vec::as_slice).collect();
    let (session, _) = fresh(12, &setup).await;

    let mut found = Vec::new();
    let mut cursor = "0".to_owned();
    loop {
        let page = session
            .scan_keys(
                12,
                &ScanRequest {
                    pattern: "product:*".into(),
                    kind: None,
                    cursor,
                    count: 50,
                },
            )
            .await
            .unwrap();
        found.extend(page.keys);
        cursor = page.cursor;
        if cursor == "0" {
            break;
        }
    }
    found.sort_by(|a, b| a.key.name.cmp(&b.key.name));
    found.dedup_by(|a, b| a.key == b.key);
    assert_eq!(found.len(), 120);
    assert!(
        found
            .iter()
            .all(|k| k.kind == "string" && k.ttl_ms.is_none())
    );

    let users = session
        .scan_keys(
            12,
            &ScanRequest {
                pattern: "user:*".into(),
                kind: Some("string".into()),
                cursor: "0".into(),
                count: 100,
            },
        )
        .await
        .unwrap();
    assert_eq!(users.keys.len(), 1);
    assert_eq!(users.keys[0].key.name, "user:1:token");
    assert!(users.keys[0].ttl_ms.is_some_and(|ms| ms > 290_000));
    session.close().await;
}

#[tokio::test]
async fn reads_every_type_and_binary_keys() {
    if !selected(REDIS) {
        return;
    }
    let (session, mut conn) = fresh(
        13,
        &[
            &["SET", "s", "{\"a\":1}"],
            &["HSET", "h", "b", "2", "a", "1"],
            &["RPUSH", "l", "x", "y"],
            &["SADD", "set", "m2", "m1"],
            &["ZADD", "z", "1.5", "one", "-3", "minus"],
            &["XADD", "x", "1-1", "status", "paid", "amount", "9.5"],
        ],
    )
    .await;
    let _: () = redis::cmd("SET")
        .arg(&[0xff_u8, 0x00][..])
        .arg(&[0xc3_u8][..])
        .query_async(&mut conn)
        .await
        .unwrap();

    let detail = session.read_key(13, &id("s")).await.unwrap();
    assert_eq!(detail.kind, "string");
    assert_eq!(detail.length, Some(7));
    assert_eq!(
        detail.value,
        KeyValue::Text {
            text: "{\"a\":1}".into(),
            truncated: false
        }
    );

    let hash = session.read_key(13, &id("h")).await.unwrap();
    assert_eq!(
        table(&hash.value).rows,
        [vec![text("a"), text("1")], vec![text("b"), text("2")]],
        "按字段排好"
    );
    let list = session.read_key(13, &id("l")).await.unwrap();
    assert_eq!(
        table(&list.value).rows[1],
        vec![CellValue::Int(1), text("y")]
    );
    let set = session.read_key(13, &id("set")).await.unwrap();
    assert_eq!(table(&set.value).rows, [vec![text("m1")], vec![text("m2")]]);
    let zset = session.read_key(13, &id("z")).await.unwrap();
    assert_eq!(
        table(&zset.value).rows,
        [
            vec![text("minus"), CellValue::Decimal("-3".into())],
            vec![text("one"), CellValue::Decimal("1.5".into())]
        ]
    );
    let stream = session.read_key(13, &id("x")).await.unwrap();
    assert_eq!(
        table(&stream.value).rows,
        [vec![
            text("1-1"),
            CellValue::Json(r#"{"status":"paid","amount":"9.5"}"#.into())
        ]]
    );
    assert!(table(&stream.value).read_only_reason.is_some());

    let binary = KeyName::from_bytes(&[0xff, 0x00]);
    assert_eq!(binary.name, "0xff00");
    let detail = session.read_key(13, &binary.id).await.unwrap();
    assert_eq!(
        detail.value,
        KeyValue::Bytes {
            hex: "c3".into(),
            truncated: false
        }
    );
    assert_eq!(
        session.read_key(13, &id("gone")).await.unwrap_err().code(),
        "not_found"
    );
    session.close().await;
}

#[tokio::test]
async fn hash_edits_are_checked_and_all_or_nothing() {
    if !selected(REDIS) {
        return;
    }
    let (session, mut conn) = fresh(
        14,
        &[&["HSET", "user", "name", "黄勇", "city", "杭州", "vip", "0"]],
    )
    .await;
    let changes = vec![
        RowChange::Update {
            key: row(&[("field", text("name")), ("value", text("黄勇"))]),
            values: values(&[("value", "黄 勇")]),
        },
        RowChange::Update {
            key: row(&[("field", text("city")), ("value", text("杭州"))]),
            values: values(&[("field", "home")]),
        },
        RowChange::Delete {
            key: row(&[("field", text("vip")), ("value", text("0"))]),
        },
        RowChange::Insert {
            values: values(&[("field", "email"), ("value", "a@b.c")]),
        },
    ];
    let preview = session
        .preview_key_changes(&id("user"), "hash", &changes)
        .unwrap();
    let outcome = session
        .apply_key_changes(14, &id("user"), "hash", &changes, Some(&preview))
        .await
        .unwrap();
    assert_eq!(outcome.statements, preview.len());
    let stored: BTreeMap<String, String> = redis::cmd("HGETALL")
        .arg("user")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        stored,
        BTreeMap::from([
            ("name".into(), "黄 勇".into()),
            ("home".into(), "杭州".into()),
            ("email".into(), "a@b.c".into()),
        ])
    );

    // 别人在中间改了 name:整批都不提交,email2 也没加上。
    let _: () = redis::cmd("HSET")
        .arg("user")
        .arg("name")
        .arg("别人改的")
        .query_async(&mut conn)
        .await
        .unwrap();
    let stale = vec![
        RowChange::Update {
            key: row(&[("field", text("name")), ("value", text("黄 勇"))]),
            values: values(&[("value", "我改的")]),
        },
        RowChange::Insert {
            values: values(&[("field", "email2"), ("value", "x")]),
        },
    ];
    let error = session
        .apply_key_changes(14, &id("user"), "hash", &stale, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "row_mismatch", "{error}");
    assert!(error.to_string().contains("字段 name"), "{error}");
    let name: String = redis::cmd("HGET")
        .arg("user")
        .arg("name")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(name, "别人改的");
    let email2: Option<String> = redis::cmd("HGET")
        .arg("user")
        .arg("email2")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(email2, None);

    // 预览之后要执行的命令变了:不执行。
    let error = session
        .apply_key_changes(
            14,
            &id("user"),
            "hash",
            &stale,
            Some(&["HSET user x y".into()]),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("重新预览"), "{error}");
    session.close().await;
}

#[tokio::test]
async fn list_set_and_zset_edits_land_where_they_should() {
    if !selected(REDIS) {
        return;
    }
    let (session, mut conn) = fresh(
        15,
        &[
            &["RPUSH", "l", "a", "b", "c", "d"],
            &["SADD", "s", "x", "y"],
            &["ZADD", "z", "1", "one", "2", "two"],
            &["EXPIRE", "l", "600"],
        ],
    )
    .await;
    let index = |i: i64| CellValue::Int(i);
    let list = vec![
        RowChange::Update {
            key: row(&[("index", index(0)), ("value", text("a"))]),
            values: values(&[("value", "A")]),
        },
        RowChange::Delete {
            key: row(&[("index", index(1)), ("value", text("b"))]),
        },
        RowChange::Delete {
            key: row(&[("index", index(3)), ("value", text("d"))]),
        },
        RowChange::Insert {
            values: values(&[("value", "e")]),
        },
    ];
    session
        .apply_key_changes(15, &id("l"), "list", &list, None)
        .await
        .unwrap();
    let items: Vec<String> = redis::cmd("LRANGE")
        .arg("l")
        .arg(0)
        .arg(-1)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(items, ["A", "c", "e"]);
    let ttl: i64 = redis::cmd("TTL")
        .arg("l")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(ttl > 590, "改元素不动过期时间");

    let set = vec![
        RowChange::Update {
            key: row(&[("member", text("x"))]),
            values: values(&[("member", "X")]),
        },
        RowChange::Insert {
            values: values(&[("member", "y")]),
        },
    ];
    let error = session
        .apply_key_changes(15, &id("s"), "set", &set, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("已经有了"), "{error}");
    let set = &set[..1];
    session
        .apply_key_changes(15, &id("s"), "set", set, None)
        .await
        .unwrap();
    let mut members: Vec<String> = redis::cmd("SMEMBERS")
        .arg("s")
        .query_async(&mut conn)
        .await
        .unwrap();
    members.sort();
    assert_eq!(members, ["X", "y"]);

    let zset = vec![
        RowChange::Update {
            key: row(&[
                ("member", text("one")),
                ("score", CellValue::Decimal("1".into())),
            ]),
            values: values(&[("score", "10.5")]),
        },
        RowChange::Update {
            key: row(&[
                ("member", text("two")),
                ("score", CellValue::Decimal("2".into())),
            ]),
            values: values(&[("member", "deux")]),
        },
    ];
    session
        .apply_key_changes(15, &id("z"), "zset", &zset, None)
        .await
        .unwrap();
    let scores: Vec<(String, f64)> = redis::cmd("ZRANGE")
        .arg("z")
        .arg(0)
        .arg(-1)
        .arg("WITHSCORES")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(scores, [("deux".to_owned(), 2.0), ("one".to_owned(), 10.5)]);
    session.close().await;
}

#[tokio::test]
async fn strings_keep_their_ttl_and_refuse_stale_writes() {
    if !selected(REDIS) {
        return;
    }
    let (session, mut conn) = fresh(10, &[&["SET", "s", "old", "EX", "600"]]).await;
    session
        .set_string(10, &id("s"), "old", "new")
        .await
        .unwrap();
    let value: String = redis::cmd("GET")
        .arg("s")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(value, "new");
    let ttl: i64 = redis::cmd("TTL")
        .arg("s")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(ttl > 590, "剩余时间不变：{ttl}");

    let error = session
        .set_string(10, &id("s"), "old", "again")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "row_mismatch", "{error}");
    session.close().await;
}

#[tokio::test]
async fn creates_renames_expires_and_deletes_keys() {
    if !selected(REDIS) {
        return;
    }
    let (session, mut conn) = fresh(8, &[&["SET", "taken", "1"]]).await;
    let new = |name: &str, kind: &str| NewKey {
        name: name.into(),
        kind: kind.into(),
        ttl_seconds: None,
        field: Some("f".into()),
        value: "v".into(),
        score: Some("1".into()),
    };
    for kind in ["string", "hash", "list", "set", "zset", "stream"] {
        let created = session
            .create_key(8, &new(&format!("new:{kind}"), kind))
            .await
            .unwrap();
        assert_eq!(session.read_key(8, &created.id).await.unwrap().kind, kind);
    }
    let error = session
        .create_key(8, &new("taken", "string"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "already_exists", "{error}");

    let renamed = session
        .rename_key(8, &id("new:string"), "renamed")
        .await
        .unwrap();
    assert_eq!(renamed.name, "renamed");
    let error = session
        .rename_key(8, &id("renamed"), "taken")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "already_exists", "{error}");

    session.set_ttl(8, &id("renamed"), Some(120)).await.unwrap();
    let ttl: i64 = redis::cmd("TTL")
        .arg("renamed")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!((110..=120).contains(&ttl));
    session.set_ttl(8, &id("renamed"), None).await.unwrap();
    let ttl: i64 = redis::cmd("TTL")
        .arg("renamed")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(ttl, -1);
    assert_eq!(
        session
            .set_ttl(8, &id("missing"), None)
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );

    let deleted = session
        .delete_keys(8, &[id("renamed"), id("new:hash"), id("missing")])
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    session.close().await;
}

#[tokio::test]
async fn renaming_the_only_member_keeps_the_expiry() {
    if !selected(REDIS) {
        return;
    }
    // 先删后加时集合会在中途变空、键被删掉;再加进去的键不能变成不过期。
    let (session, mut conn) = fresh(
        6,
        &[
            &["HSET", "h", "token", "abc"],
            &["SADD", "s", "only"],
            &["ZADD", "z", "1", "only"],
            &["EXPIRE", "h", "600"],
            &["EXPIRE", "s", "600"],
            &["EXPIRE", "z", "600"],
        ],
    )
    .await;
    let renames = [
        (
            "h",
            "hash",
            RowChange::Update {
                key: row(&[("field", text("token")), ("value", text("abc"))]),
                values: values(&[("field", "jwt")]),
            },
        ),
        (
            "s",
            "set",
            RowChange::Update {
                key: row(&[("member", text("only"))]),
                values: values(&[("member", "renamed")]),
            },
        ),
        (
            "z",
            "zset",
            RowChange::Update {
                key: row(&[
                    ("member", text("only")),
                    ("score", CellValue::Decimal("1".into())),
                ]),
                values: values(&[("member", "renamed")]),
            },
        ),
    ];
    for (key, kind, change) in renames {
        session
            .apply_key_changes(6, &id(key), kind, &[change], None)
            .await
            .unwrap();
        let ttl: i64 = redis::cmd("TTL")
            .arg(key)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(ttl > 590, "{key} 改名之后剩余时间不变：{ttl}");
    }
    session.close().await;
}
