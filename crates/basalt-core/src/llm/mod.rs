//! SQL 助手要用的大模型接口:用户自己配置模型(OpenAI 兼容格式或 Claude 格式),流式读回答。
//!
//! 助手只生成、解释、优化 SQL,不执行:这里只负责把提示词发出去、把回答一段段交给调用方。
//! 发给模型的是表结构(表名、列名、类型)和用户写的问题 / SQL,由前端拼好;不含表里的数据。

pub mod client;
pub mod config;
pub mod outline;
pub mod protocol;
mod sse;

use serde::{Deserialize, Serialize};

/// 接口格式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    /// OpenAI 的 Chat Completions,以及兼容它的各家接口与网关。
    Openai,
    /// Anthropic 的 Messages(Claude)。
    Anthropic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

/// 一次对话请求:用哪个模型、系统提示词、到目前为止的对话(最后一条是用户的新问题)。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub model_id: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
}
