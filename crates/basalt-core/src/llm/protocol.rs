//! 两种接口格式的请求怎么拼、流式响应怎么读。纯函数,不碰网络。
//!
//! - OpenAI 兼容:`POST {base}/chat/completions`,`Authorization: Bearer`,流里每个 `data:` 是一段
//!   `choices[0].delta.content`,以 `data: [DONE]` 结束。DeepSeek、通义、Ollama、各家网关都是这个格式。
//! - Claude(Anthropic Messages):`POST {base}/v1/messages`,`x-api-key` + `anthropic-version`,
//!   文字在 `content_block_delta` 事件的 `delta.text` 里,以 `message_stop` 结束。

use serde_json::{Value, json};

use super::config::LlmModel;
use super::sse::SseEvent;
use super::{ChatMessage, Protocol, Role};

/// Claude 要求必须给 max_tokens;用户没填时用这个。
pub const DEFAULT_MAX_TOKENS: u32 = 4096;
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, PartialEq)]
pub(crate) struct HttpRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

/// 流里读出来的一项。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StreamItem {
    Text(String),
    Done,
    /// 心跳、元数据、思考过程这类不展示的事件。
    Skip,
}

/// 用户填的 base_url 补成完整的接口地址。填了完整地址的原样用。
pub fn endpoint(protocol: Protocol, base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    match protocol {
        Protocol::Openai => {
            if base.ends_with("/chat/completions") {
                base.to_owned()
            } else if has_path(base) {
                format!("{base}/chat/completions")
            } else {
                // 只填了域名(https://api.openai.com):补上 /v1。
                format!("{base}/v1/chat/completions")
            }
        }
        Protocol::Anthropic => {
            if base.ends_with("/messages") {
                base.to_owned()
            } else if base.ends_with("/v1") {
                format!("{base}/messages")
            } else {
                format!("{base}/v1/messages")
            }
        }
    }
}

/// `https://host` 没有路径;`https://host/v1`、`http://host:11434/api` 有。
fn has_path(base: &str) -> bool {
    let after_scheme = base.split_once("://").map_or(base, |(_, rest)| rest);
    after_scheme.contains('/')
}

fn role(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

pub(crate) fn build(
    model: &LlmModel,
    api_key: Option<&str>,
    system: &str,
    messages: &[ChatMessage],
    stream: bool,
) -> HttpRequest {
    let key = api_key.filter(|k| !k.trim().is_empty());
    let turns = messages
        .iter()
        .map(|m| json!({ "role": role(m.role), "content": m.content }));
    match model.protocol {
        Protocol::Openai => {
            let mut body = json!({
                "model": model.model,
                "messages": std::iter::once(json!({ "role": "system", "content": system }))
                    .chain(turns)
                    .collect::<Vec<_>>(),
                "stream": stream,
            });
            if let Some(max) = model.max_tokens {
                body["max_tokens"] = json!(max);
            }
            if let Some(temperature) = model.temperature {
                body["temperature"] = json!(temperature);
            }
            let mut headers = vec![("content-type", "application/json".to_owned())];
            if let Some(key) = key {
                headers.push(("authorization", format!("Bearer {key}")));
            }
            HttpRequest {
                url: endpoint(Protocol::Openai, &model.base_url),
                headers,
                body,
            }
        }
        Protocol::Anthropic => {
            let mut body = json!({
                "model": model.model,
                "system": system,
                "messages": turns.collect::<Vec<_>>(),
                "max_tokens": model.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
                "stream": stream,
            });
            if let Some(temperature) = model.temperature {
                body["temperature"] = json!(temperature);
            }
            let mut headers = vec![
                ("content-type", "application/json".to_owned()),
                ("anthropic-version", ANTHROPIC_VERSION.to_owned()),
            ];
            if let Some(key) = key {
                headers.push(("x-api-key", key.to_owned()));
            }
            HttpRequest {
                url: endpoint(Protocol::Anthropic, &model.base_url),
                headers,
                body,
            }
        }
    }
}

/// 一个流式事件读成一项;事件里报的错作为 `Err` 返回。
pub(crate) fn decode(protocol: Protocol, event: &SseEvent) -> Result<StreamItem, String> {
    let data = event.data.trim();
    if data.is_empty() {
        return Ok(StreamItem::Skip);
    }
    if protocol == Protocol::Openai && data == "[DONE]" {
        return Ok(StreamItem::Done);
    }
    let value: Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(_) => return Ok(StreamItem::Skip),
    };
    if let Some(message) = error_in(&value) {
        return Err(message);
    }
    match protocol {
        Protocol::Openai => {
            let text = value["choices"][0]["delta"]["content"]
                .as_str()
                .unwrap_or("");
            Ok(if text.is_empty() {
                StreamItem::Skip
            } else {
                StreamItem::Text(text.to_owned())
            })
        }
        Protocol::Anthropic => {
            let kind = event
                .event
                .as_deref()
                .or_else(|| value["type"].as_str())
                .unwrap_or("");
            match kind {
                "content_block_delta" if value["delta"]["type"] == "text_delta" => Ok(
                    StreamItem::Text(value["delta"]["text"].as_str().unwrap_or("").to_owned()),
                ),
                "message_stop" => Ok(StreamItem::Done),
                _ => Ok(StreamItem::Skip),
            }
        }
    }
}

/// 不是流式的一整个回答(有的网关忽略 stream:true)里的文字。
pub(crate) fn whole_text(protocol: Protocol, value: &Value) -> Option<String> {
    match protocol {
        Protocol::Openai => value["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned),
        Protocol::Anthropic => {
            let parts: Vec<&str> = value["content"]
                .as_array()?
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect();
            Some(parts.concat())
        }
    }
}

/// 两家的错误都形如 `{"error": {"message": …}}`;有的网关 error 直接是字符串。
fn error_in(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if error.is_null() {
        return None;
    }
    Some(
        error["message"]
            .as_str()
            .or_else(|| error.as_str())
            .map_or_else(|| error.to_string(), str::to_owned),
    )
}

/// HTTP 出错时给人看的一句话:能从响应里读出错误信息就用它,否则带上状态码和响应开头。
pub(crate) fn http_error(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| error_in(&v))
        .unwrap_or_else(|| body.chars().take(300).collect::<String>().trim().to_owned());
    let hint = match status {
        401 | 403 => "，请检查 API Key",
        404 => "，请检查 base_url 和模型名称",
        429 => "，请求太频繁或额度用完了",
        _ => "",
    };
    if detail.is_empty() {
        format!("HTTP {status}{hint}")
    } else {
        format!("HTTP {status}{hint}：{detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(protocol: Protocol, base_url: &str) -> LlmModel {
        LlmModel {
            id: "m1".into(),
            name: "测试".into(),
            protocol,
            base_url: base_url.into(),
            model: "gpt-x".into(),
            max_tokens: None,
            temperature: None,
            has_api_key: true,
        }
    }

    fn sse(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    #[test]
    fn base_urls_become_full_endpoints() {
        use Protocol::*;
        for (protocol, base, url) in [
            (
                Openai,
                "https://api.openai.com",
                "https://api.openai.com/v1/chat/completions",
            ),
            (
                Openai,
                "https://api.openai.com/v1/",
                "https://api.openai.com/v1/chat/completions",
            ),
            (
                Openai,
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
            ),
            (
                Openai,
                "http://127.0.0.1:11434/v1/chat/completions",
                "http://127.0.0.1:11434/v1/chat/completions",
            ),
            (
                Anthropic,
                "https://api.anthropic.com",
                "https://api.anthropic.com/v1/messages",
            ),
            (
                Anthropic,
                "https://gw.example.com/v1",
                "https://gw.example.com/v1/messages",
            ),
            (
                Anthropic,
                "https://gw.example.com/claude/v1/messages",
                "https://gw.example.com/claude/v1/messages",
            ),
        ] {
            assert_eq!(endpoint(protocol, base), url, "{base}");
        }
    }

    #[test]
    fn openai_requests_put_the_system_prompt_first() {
        let mut m = model(Protocol::Openai, "https://api.openai.com/v1");
        m.max_tokens = Some(800);
        m.temperature = Some(0.2);
        let messages = [ChatMessage {
            role: Role::User,
            content: "查最近的订单".into(),
        }];
        let request = build(&m, Some("sk-1"), "你是 SQL 助手", &messages, true);
        assert_eq!(request.url, "https://api.openai.com/v1/chat/completions");
        assert!(
            request
                .headers
                .contains(&("authorization", "Bearer sk-1".into()))
        );
        assert_eq!(
            request.body,
            json!({
                "model": "gpt-x",
                "messages": [
                    { "role": "system", "content": "你是 SQL 助手" },
                    { "role": "user", "content": "查最近的订单" },
                ],
                "stream": true,
                "max_tokens": 800,
                "temperature": 0.2,
            })
        );
        // 本地模型(Ollama)可以不要密钥。
        let request = build(&m, Some("  "), "s", &messages, true);
        assert!(
            !request
                .headers
                .iter()
                .any(|(name, _)| *name == "authorization")
        );
    }

    #[test]
    fn claude_requests_use_the_messages_api() {
        let m = model(Protocol::Anthropic, "https://api.anthropic.com");
        let messages = [
            ChatMessage {
                role: Role::User,
                content: "a".into(),
            },
            ChatMessage {
                role: Role::Assistant,
                content: "b".into(),
            },
            ChatMessage {
                role: Role::User,
                content: "c".into(),
            },
        ];
        let request = build(&m, Some("sk-ant"), "系统", &messages, true);
        assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
        assert!(request.headers.contains(&("x-api-key", "sk-ant".into())));
        assert!(
            request
                .headers
                .contains(&("anthropic-version", ANTHROPIC_VERSION.into()))
        );
        assert_eq!(request.body["system"], "系统");
        assert_eq!(request.body["max_tokens"], DEFAULT_MAX_TOKENS);
        assert_eq!(
            request.body["messages"][1],
            json!({ "role": "assistant", "content": "b" })
        );
        assert!(request.body.get("temperature").is_none());
    }

    #[test]
    fn openai_stream_events() {
        let p = Protocol::Openai;
        assert_eq!(
            decode(
                p,
                &sse(None, r#"{"choices":[{"delta":{"content":"SELECT"}}]}"#)
            ),
            Ok(StreamItem::Text("SELECT".into()))
        );
        assert_eq!(
            decode(
                p,
                &sse(None, r#"{"choices":[{"delta":{"role":"assistant"}}]}"#)
            ),
            Ok(StreamItem::Skip)
        );
        assert_eq!(decode(p, &sse(None, "[DONE]")), Ok(StreamItem::Done));
        assert_eq!(
            decode(p, &sse(None, r#"{"error":{"message":"model not found"}}"#)),
            Err("model not found".into())
        );
    }

    #[test]
    fn claude_stream_events() {
        let p = Protocol::Anthropic;
        let delta = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"查询"}}"#;
        assert_eq!(
            decode(p, &sse(Some("content_block_delta"), delta)),
            Ok(StreamItem::Text("查询".into()))
        );
        // 没有 event 行的网关:看 data 里的 type。
        assert_eq!(
            decode(p, &sse(None, delta)),
            Ok(StreamItem::Text("查询".into()))
        );
        let thinking =
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"…"}}"#;
        assert_eq!(
            decode(p, &sse(Some("content_block_delta"), thinking)),
            Ok(StreamItem::Skip)
        );
        assert_eq!(
            decode(p, &sse(Some("ping"), r#"{"type":"ping"}"#)),
            Ok(StreamItem::Skip)
        );
        assert_eq!(
            decode(p, &sse(Some("message_stop"), r#"{"type":"message_stop"}"#)),
            Ok(StreamItem::Done)
        );
        assert_eq!(
            decode(
                p,
                &sse(
                    Some("error"),
                    r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
                )
            ),
            Err("Overloaded".into())
        );
    }

    #[test]
    fn whole_answers_and_http_errors() {
        assert_eq!(
            whole_text(
                Protocol::Openai,
                &json!({"choices":[{"message":{"content":"ok"}}]})
            )
            .as_deref(),
            Some("ok")
        );
        assert_eq!(
            whole_text(
                Protocol::Anthropic,
                &json!({"content":[{"type":"text","text":"o"},{"type":"text","text":"k"}]})
            )
            .as_deref(),
            Some("ok")
        );
        assert_eq!(
            http_error(401, r#"{"error":{"message":"Incorrect API key"}}"#),
            "HTTP 401，请检查 API Key：Incorrect API key"
        );
        assert_eq!(
            http_error(502, "<html>Bad Gateway</html>"),
            "HTTP 502：<html>Bad Gateway</html>"
        );
        assert_eq!(http_error(404, ""), "HTTP 404，请检查 base_url 和模型名称");
    }
}
