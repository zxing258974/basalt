//! 模型配置与钥匙串、流式对话和取消。对话对着本机的假服务器,不碰真的模型接口。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use basalt_core::llm::config::{LlmModelDraft, api_key_slot};
use basalt_core::llm::{ChatMessage, ChatRequest, Protocol, Role};
use basalt_core::secret::{MemorySecretStore, SecretStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

fn state() -> (tempfile::TempDir, AppState, Arc<MemorySecretStore>) {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let state = AppState::new(dir.path().join("connections.json"), secrets.clone());
    (dir, state, secrets)
}

fn draft(base_url: &str) -> LlmModelDraft {
    LlmModelDraft {
        id: None,
        name: "本机".into(),
        protocol: Protocol::Openai,
        base_url: base_url.into(),
        model: "test".into(),
        max_tokens: None,
        temperature: None,
        api_key: Some("sk-1".into()),
    }
}

/// 只接一次请求的假服务器;`gap` 是每段之间停多久。
async fn serve(chunks: Vec<String>, gap: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 65536];
        // 请求很小,一次读完即可。
        let _ = socket.read(&mut buffer).await;
        let head =
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
        if socket.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        for chunk in chunks {
            if socket.write_all(chunk.as_bytes()).await.is_err() {
                return;
            }
            tokio::time::sleep(gap).await;
        }
    });
    format!("http://{address}/v1")
}

fn delta(text: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n")
}

fn ask(model_id: &str) -> ChatRequest {
    ChatRequest {
        model_id: model_id.into(),
        system: "系统".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "写个查询".into(),
        }],
    }
}

#[tokio::test]
async fn keys_live_in_the_keychain_and_follow_the_model() {
    let (dir, state, secrets) = state();
    let saved = save_llm_model(&state, draft("https://api.example.com/v1"))
        .await
        .unwrap();
    assert!(saved.has_api_key);
    assert_eq!(
        secrets.get(&api_key_slot(&saved.id)).unwrap().as_deref(),
        Some("sk-1")
    );
    let file = std::fs::read_to_string(dir.path().join("llm-models.json")).unwrap();
    assert!(!file.contains("sk-1"), "Key 不写进配置文件");
    let settings = list_llm_models(&state).await.unwrap();
    assert_eq!(settings.active.as_deref(), Some(saved.id.as_str()));

    // 编辑时 Key 留空:保留钥匙串里的。
    let edit = LlmModelDraft {
        id: Some(saved.id.clone()),
        name: "改名".into(),
        api_key: None,
        ..draft("https://api.example.com/v1")
    };
    let edited = save_llm_model(&state, edit.clone()).await.unwrap();
    assert!(edited.has_api_key);
    assert_eq!(
        secrets.get(&api_key_slot(&saved.id)).unwrap().as_deref(),
        Some("sk-1")
    );

    // 清空:删掉。
    let cleared = save_llm_model(
        &state,
        LlmModelDraft {
            api_key: Some(" ".into()),
            ..edit
        },
    )
    .await
    .unwrap();
    assert!(!cleared.has_api_key);
    assert_eq!(secrets.get(&api_key_slot(&saved.id)).unwrap(), None);

    let other = save_llm_model(&state, draft("https://api.example.com/v1"))
        .await
        .unwrap();
    let settings = set_active_llm_model(&state, &other.id).await.unwrap();
    assert_eq!(settings.active.as_deref(), Some(other.id.as_str()));
    let settings = delete_llm_model(&state, &other.id).await.unwrap();
    assert_eq!(settings.active.as_deref(), Some(saved.id.as_str()));
    assert_eq!(secrets.get(&api_key_slot(&other.id)).unwrap(), None);

    let error = save_llm_model(
        &state,
        LlmModelDraft {
            id: Some("nope".into()),
            ..draft("https://api.example.com/v1")
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    let error = save_llm_model(&state, draft("not a url"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_config");
}

#[test]
fn a_cancel_that_arrives_before_the_stream_starts_still_counts() {
    let chats = LlmChats::default();
    let registered = chats.register("early").unwrap();
    assert!(chats.register("early").is_err(), "同一个 id 不能开两次");
    assert!(chats.cancel("early"));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let task = runtime.spawn(std::future::pending::<()>());
    assert!(
        !chats.attach("early", task.abort_handle()),
        "已经取消了,不该开始"
    );
    task.abort();
    drop(registered);
    assert!(!chats.cancel("early"), "结束后登记就去掉了");
}

#[tokio::test]
async fn changing_the_host_requires_the_key_again() {
    let (_dir, state, _secrets) = state();
    let saved = save_llm_model(&state, draft("https://api.example.com/v1"))
        .await
        .unwrap();
    let moved = LlmModelDraft {
        id: Some(saved.id.clone()),
        api_key: None,
        ..draft("https://gw.other.com/v1")
    };
    let error = test_llm_model(&state, moved.clone()).await.unwrap_err();
    assert_eq!(error.code, "invalid_config");
    assert_eq!(error.fields[0].field, "apiKey");
    assert_eq!(
        save_llm_model(&state, moved.clone())
            .await
            .unwrap_err()
            .code,
        "invalid_config"
    );
    // 同一个主机换路径可以;重填了 Key 也可以。
    let same_host = LlmModelDraft {
        base_url: "https://api.example.com/v2".into(),
        ..moved.clone()
    };
    assert!(save_llm_model(&state, same_host).await.is_ok());
    let with_key = LlmModelDraft {
        api_key: Some("sk-2".into()),
        ..moved
    };
    assert!(save_llm_model(&state, with_key).await.is_ok());
}

#[tokio::test]
async fn chats_stream_every_piece_and_can_be_cancelled() {
    let (_dir, state, _secrets) = state();
    let base = serve(
        vec![delta("SELECT"), delta(" 1;"), "data: [DONE]\n\n".into()],
        Duration::ZERO,
    )
    .await;
    let model = save_llm_model(&state, draft(&base)).await.unwrap();
    let pieces = Arc::new(Mutex::new(Vec::new()));
    let sink = pieces.clone();
    let outcome = llm_chat(&state, "r1", ask(&model.id), move |t| {
        sink.lock().unwrap().push(t.to_owned())
    })
    .await
    .unwrap();
    assert_eq!((outcome.chars, outcome.text.as_str()), (9, "SELECT 1;"));
    assert_eq!(*pieces.lock().unwrap(), ["SELECT", " 1;"]);
    assert!(!cancel_llm_chat(&state, "r1"), "已经结束的对话取消不了");

    // 一段一段慢慢来:第一段到了就取消。
    let slow: Vec<String> = (0..50).map(|i| delta(&format!("片{i}"))).collect();
    let base = serve(slow, Duration::from_millis(100)).await;
    let model = save_llm_model(&state, draft(&base)).await.unwrap();
    let state = Arc::new(state);
    let (first_tx, first_rx) = tokio::sync::oneshot::channel::<()>();
    let first = Mutex::new(Some(first_tx));
    let chat = {
        let state = state.clone();
        let id = model.id.clone();
        tokio::spawn(async move {
            llm_chat(&state, "r2", ask(&id), move |_| {
                if let Some(tx) = first.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            })
            .await
        })
    };
    first_rx.await.unwrap();
    assert!(cancel_llm_chat(&state, "r2"));
    let error = tokio::time::timeout(Duration::from_secs(5), chat)
        .await
        .expect("取消后应该很快结束")
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "cancelled");

    let error = llm_chat(&state, "r3", ask("gone"), |_| {})
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_found");
}
