//! 对着本机的假服务器测:两种格式的流式回答、被切开的数据块、HTTP 错误、非流式回答、连不上。
//! 不碰真的模型接口,也不走代理。

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::llm::Protocol;

struct Canned {
    status: &'static str,
    content_type: &'static str,
    /// 按字节发:可以切在一个汉字中间。
    chunks: Vec<Vec<u8>>,
}

fn sse(chunks: &[&str]) -> Canned {
    Canned {
        status: "200 OK",
        content_type: "text/event-stream",
        chunks: chunks.iter().map(|c| c.as_bytes().to_vec()).collect(),
    }
}

/// 起一个只接一次请求的服务器:返回 base_url 和「收到的请求原文」。
async fn serve(canned: Canned) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let n = socket.read(&mut buffer).await.unwrap();
            request.extend_from_slice(&buffer[..n]);
            let text = String::from_utf8_lossy(&request);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        let head = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
            canned.status, canned.content_type
        );
        socket.write_all(head.as_bytes()).await.unwrap();
        for chunk in canned.chunks {
            socket.write_all(&chunk).await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        socket.shutdown().await.ok();
        String::from_utf8_lossy(&request).into_owned()
    });
    (format!("http://{address}"), handle)
}

fn model(protocol: Protocol, base_url: &str) -> LlmModel {
    LlmModel {
        id: "m".into(),
        name: "测试".into(),
        protocol,
        base_url: base_url.into(),
        model: "test-model".into(),
        max_tokens: None,
        temperature: None,
        has_api_key: true,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

async fn chat(model: &LlmModel) -> (CoreResult<usize>, Vec<String>) {
    let mut pieces = Vec::new();
    let question = [ChatMessage {
        role: Role::User,
        content: "订单表有多少行".into(),
    }];
    let result = stream_chat(
        &client(),
        model,
        Some("sk-test"),
        "系统提示",
        &question,
        &mut |t| pieces.push(t.to_owned()),
    )
    .await;
    (result, pieces)
}

#[tokio::test]
async fn openai_answers_stream_piece_by_piece() {
    let (base, request) = serve(sse(&[
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"SELECT\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" count(*)\"}}]}\n\ndata: [DONE]\n\n",
    ]))
    .await;
    let (result, pieces) = chat(&model(Protocol::Openai, &format!("{base}/v1"))).await;
    assert_eq!(result.unwrap(), "SELECT count(*)".chars().count());
    assert_eq!(pieces, ["SELECT", " count(*)"]);
    let request = request.await.unwrap();
    assert!(
        request.starts_with("POST /v1/chat/completions HTTP/1.1"),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test")
    );
    assert!(request.contains("\"stream\":true") && request.contains("订单表有多少行"));
}

#[tokio::test]
async fn claude_answers_survive_chunks_cut_inside_a_character() {
    let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"查询订单\"}}\n\n";
    // 在「查」的第二个字节处切开。
    let cut = event.find('查').unwrap() + 1;
    let bytes = event.as_bytes();
    let mut canned = sse(&["event: message_start\ndata: {\"type\":\"message_start\"}\n\n"]);
    canned.chunks.push(bytes[..cut].to_vec());
    canned.chunks.push(bytes[cut..].to_vec());
    canned
        .chunks
        .push(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_vec());
    let (base, request) = serve(canned).await;
    let (result, pieces) = chat(&model(Protocol::Anthropic, &base)).await;
    assert_eq!(result.unwrap(), 4);
    assert_eq!(pieces, ["查询订单"]);
    let request = request.await.unwrap().to_ascii_lowercase();
    assert!(
        request.starts_with("post /v1/messages http/1.1"),
        "{request}"
    );
    assert!(
        request.contains("x-api-key: sk-test") && request.contains("anthropic-version: 2023-06-01")
    );
}

#[tokio::test]
async fn http_errors_and_stream_errors_are_explained() {
    let (base, _) = serve(Canned {
        status: "401 Unauthorized",
        content_type: "application/json",
        chunks: vec![r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.as_bytes().to_vec()],
    })
    .await;
    let error = chat(&model(Protocol::Anthropic, &base))
        .await
        .0
        .unwrap_err();
    assert_eq!(error.code(), "llm");
    assert_eq!(
        error.to_string(),
        "模型请求失败：HTTP 401，请检查 API Key：invalid x-api-key"
    );

    let (base, _) = serve(sse(&[
        "event: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"Overloaded\"}}\n\n",
    ]))
    .await;
    let error = chat(&model(Protocol::Anthropic, &base))
        .await
        .0
        .unwrap_err();
    assert_eq!(error.to_string(), "模型请求失败：Overloaded");

    let (base, _) = serve(sse(&["data: {\"choices\":[{\"delta\":{}}]}\n\n"])).await;
    let error = chat(&model(Protocol::Openai, &base)).await.0.unwrap_err();
    assert!(error.to_string().contains("没有返回内容"), "{error}");
}

#[tokio::test]
async fn redirects_are_not_followed_and_empty_answers_are_errors() {
    let (base, _) = serve(Canned {
        status: "302 Found\r\nLocation: https://sso.example.com/login",
        content_type: "text/html",
        chunks: Vec::new(),
    })
    .await;
    // 用和应用里一样的客户端:它不跟跳转。
    let mut pieces = Vec::new();
    let question = [ChatMessage {
        role: Role::User,
        content: "q".into(),
    }];
    let error = stream_chat(
        shared_client().unwrap(),
        &model(Protocol::Anthropic, &base),
        Some("sk-test"),
        "s",
        &question,
        &mut |t| pieces.push(t.to_owned()),
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("https://sso.example.com/login"),
        "{error}"
    );
    assert!(pieces.is_empty());

    // 收到了 message_stop 却一个字都没有。
    let (base, _) = serve(sse(&[
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]))
    .await;
    let error = chat(&model(Protocol::Anthropic, &base))
        .await
        .0
        .unwrap_err();
    assert!(
        error.to_string().contains("最多输出 tokens 太小"),
        "{error}"
    );
}

#[tokio::test]
async fn gateways_that_ignore_streaming_still_work() {
    let (base, _) = serve(Canned {
        status: "200 OK",
        content_type: "application/json",
        chunks: vec![
            r#"{"choices":[{"message":{"role":"assistant","content":"SELECT 1;"}}]}"#.into(),
        ],
    })
    .await;
    let (result, pieces) = chat(&model(Protocol::Openai, &base)).await;
    assert_eq!(result.unwrap(), 9);
    assert_eq!(pieces, ["SELECT 1;"]);

    // 流没有标 text/event-stream,最后也没有 [DONE]。
    let (base, _) = serve(Canned {
        status: "200 OK",
        content_type: "text/plain",
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n"
                .as_bytes()
                .to_vec(),
        ],
    })
    .await;
    assert_eq!(chat(&model(Protocol::Openai, &base)).await.1, ["ok"]);
}

#[tokio::test]
async fn unreachable_servers_say_so_and_tests_report_the_reply() {
    // 先占一个端口再放掉:这时没人在听。
    let address = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let error = chat(&model(Protocol::Openai, &format!("http://{address}")))
        .await
        .0
        .unwrap_err();
    assert!(error.to_string().contains("连不上服务器"), "{error}");

    let (base, _) = serve(sse(&[
        "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\ndata: [DONE]\n\n",
    ]))
    .await;
    let reply = test_model(&client(), &model(Protocol::Openai, &base), None)
        .await
        .unwrap();
    assert_eq!(reply.reply, "OK");
}
