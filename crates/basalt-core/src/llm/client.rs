//! 发请求、读流。只请求用户配置的地址;走 macOS 的系统代理(如果设了)和 HTTPS_PROXY 这类环境变量。
//! 取消由调用方丢弃这个 future(中止任务)完成:连接随之断开。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;

use super::config::LlmModel;
use super::protocol::{self, StreamItem};
use super::sse::{SseEvent, SseParser};
use super::{ChatMessage, Role};
use crate::error::{CoreError, CoreResult};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 两块数据之间最多等这么久;模型想得久的时候也会先发心跳。
const READ_TIMEOUT: Duration = Duration::from_secs(180);

/// 整个应用共用一个客户端,连接可以复用。
pub fn shared_client() -> CoreResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
                // 不跟跳转:reqwest 跨域跳转时只去掉 Authorization,x-api-key 会被带到别的地址。
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| CoreError::Llm(format!("创建 HTTP 客户端失败：{e}")))
}

/// reqwest 的错误只有最外一层的说明,把底下的原因(DNS、TLS、拒绝连接……)一起带上。
fn describe(error: &reqwest::Error) -> String {
    let mut text = if error.is_timeout() {
        "等待响应超时".to_owned()
    } else if error.is_connect() {
        "连不上服务器".to_owned()
    } else {
        "请求失败".to_owned()
    };
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push('：');
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// 一个流式事件:文字交给 `on_text`;返回是否已经结束。
fn absorb(
    model: &LlmModel,
    event: &SseEvent,
    on_text: &mut (dyn FnMut(&str) + Send),
    total: &mut usize,
) -> CoreResult<bool> {
    match protocol::decode(model.protocol, event).map_err(CoreError::Llm)? {
        StreamItem::Text(text) => {
            *total += text.chars().count();
            on_text(&text);
            Ok(false)
        }
        StreamItem::Done => {
            if *total == 0 {
                return Err(CoreError::Llm(EMPTY_ANSWER.into()));
            }
            Ok(true)
        }
        StreamItem::Skip => Ok(false),
    }
}

/// 收到了结束标记,却一个字都没有:多半是 max_tokens 太小(推理模型想完就没额度了)或被内容过滤。
const EMPTY_ANSWER: &str = "模型没有返回内容（可能是最多输出 tokens 太小，或者回答被过滤了）。";

/// 发一次对话,回答一段段交给 `on_text`,返回一共多少字。
pub async fn stream_chat(
    client: &reqwest::Client,
    model: &LlmModel,
    api_key: Option<&str>,
    system: &str,
    messages: &[ChatMessage],
    on_text: &mut (dyn FnMut(&str) + Send),
) -> CoreResult<usize> {
    let request = protocol::build(model, api_key, system, messages, true);
    // Content-Type 已经在请求头里,body 直接发序列化好的 JSON。
    let mut builder = client.post(&request.url).body(request.body.to_string());
    for (name, value) in &request.headers {
        builder = builder.header(*name, value);
    }
    let response = builder
        .send()
        .await
        .map_err(|e| CoreError::Llm(format!("{}（{}）", describe(&e), request.url)))?;
    let status = response.status();
    if status.is_redirection() {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("（没给地址）");
        return Err(CoreError::Llm(format!(
            "服务器要求跳转到 {location}。为了不把 API Key 带到别的地址，没有跟过去；如果这是正确的接口地址，请把 Base URL 改成它。"
        )));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(CoreError::Llm(protocol::http_error(status.as_u16(), &body)));
    }
    let event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/event-stream"));
    let mut total = 0;
    let mut parser = SseParser::default();
    if !event_stream {
        // 有的网关忽略 stream:true,一次给出整个回答;也有的流没标对 Content-Type。
        let body = response
            .text()
            .await
            .map_err(|e| CoreError::Llm(describe(&e)))?;
        let trimmed = body.trim_start();
        if !(trimmed.starts_with("data:") || trimmed.starts_with("event:")) {
            let value: Value = serde_json::from_str(&body).map_err(|_| {
                CoreError::Llm(format!(
                    "响应既不是流也不是 JSON：{}",
                    body.chars().take(200).collect::<String>()
                ))
            })?;
            if let Some(message) = value["error"]["message"].as_str() {
                return Err(CoreError::Llm(message.to_owned()));
            }
            let text = protocol::whole_text(model.protocol, &value)
                .ok_or_else(|| CoreError::Llm("响应里没有回答的文字。".into()))?;
            if text.trim().is_empty() {
                return Err(CoreError::Llm(EMPTY_ANSWER.into()));
            }
            on_text(&text);
            return Ok(text.chars().count());
        }
        for event in parser
            .feed(body.as_bytes())
            .iter()
            .chain(parser.finish().iter())
        {
            if absorb(model, event, on_text, &mut total)? {
                return Ok(total);
            }
        }
        return finished(total);
    }
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk =
            chunk.map_err(|e| CoreError::Llm(format!("读回答时断开了：{}", describe(&e))))?;
        for event in parser.feed(&chunk) {
            if absorb(model, &event, on_text, &mut total)? {
                return Ok(total);
            }
        }
    }
    if let Some(event) = parser.finish()
        && absorb(model, &event, on_text, &mut total)?
    {
        return Ok(total);
    }
    finished(total)
}

/// 流结束了却没收到结束标记:有内容就算完成(有的服务直接断开),一个字都没有就是出错了。
fn finished(total: usize) -> CoreResult<usize> {
    if total == 0 {
        Err(CoreError::Llm(EMPTY_ANSWER.into()))
    } else {
        Ok(total)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReply {
    /// 模型回的话(最多 200 字)。
    pub reply: String,
    pub millis: u64,
}

/// 「测试连接」:发一句很短的话,看能不能流式读回答案。
pub async fn test_model(
    client: &reqwest::Client,
    model: &LlmModel,
    api_key: Option<&str>,
) -> CoreResult<TestReply> {
    let started = Instant::now();
    let mut reply = String::new();
    let messages = [ChatMessage {
        role: Role::User,
        content: "这是连通性测试，请只回复 OK。".into(),
    }];
    stream_chat(
        client,
        model,
        api_key,
        "你是连通性测试助手。",
        &messages,
        &mut |text| {
            if reply.chars().count() < 200 {
                reply.push_str(text);
            }
        },
    )
    .await?;
    Ok(TestReply {
        reply: reply.chars().take(200).collect(),
        millis: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
