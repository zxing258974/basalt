//! SQL 助手:模型配置(Key 进密码存储)、流式对话、表结构概要。逻辑在 core 的 `llm` 里。
//!
//! 对话由前端起一个请求 id:命令还没返回时也能按它取消。取消就是中止那个任务,
//! 正在读的 HTTP 响应随之丢弃、连接断开。

use std::collections::HashMap;
use std::sync::Mutex;

use basalt_core::llm::ChatRequest;
use basalt_core::llm::client::{TestReply, shared_client, stream_chat, test_model};
use basalt_core::llm::config::{LlmModel, LlmModelDraft, LlmSettings, api_key_slot, same_origin};
use basalt_core::llm::outline::OutlineTable;
use basalt_core::secret::QuietDelete;
use basalt_core::{CoreError, FieldError};
use serde::Serialize;
use tokio::task::AbortHandle;

use super::CommandResult;
use crate::state::AppState;

/// 一个对话:命令一进来就登记(这时还在读配置、密码存储),任务开始后再挂上中止句柄。
/// 登记之后、开始之前来的取消只记下来,开始时看到就不开始。
#[derive(Default)]
struct Chat {
    cancelled: bool,
    handle: Option<AbortHandle>,
}

/// 正在进行的对话,取消时按请求 id 找。
#[derive(Default)]
pub struct LlmChats {
    running: Mutex<HashMap<String, Chat>>,
}

/// 对话结束(不管怎么结束)时把登记去掉。
struct Registered<'a> {
    chats: &'a LlmChats,
    id: String,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        self.chats.running().remove(&self.id);
    }
}

impl LlmChats {
    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<String, Chat>> {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn register(&self, id: &str) -> CommandResult<Registered<'_>> {
        let mut running = self.running();
        if running.contains_key(id) {
            return Err(CoreError::AlreadyExists(format!("对话 {id} 已经在进行中。")).into());
        }
        running.insert(id.to_owned(), Chat::default());
        Ok(Registered {
            chats: self,
            id: id.to_owned(),
        })
    }

    /// 挂上中止句柄;在这之前已经被取消的返回 false。
    fn attach(&self, id: &str, handle: AbortHandle) -> bool {
        match self.running().get_mut(id) {
            Some(chat) if !chat.cancelled => {
                chat.handle = Some(handle);
                true
            }
            _ => false,
        }
    }

    /// 没有这个对话(已经结束)时返回 false。
    fn cancel(&self, id: &str) -> bool {
        match self.running().get_mut(id) {
            Some(chat) => {
                chat.cancelled = true;
                if let Some(handle) = &chat.handle {
                    handle.abort();
                }
                true
            }
            None => false,
        }
    }
}

/// 编辑时换了接口的主机却没重填 Key:原来的 Key 不发给新地址,要用户重新填。
fn check_key_origin(existing: Option<&LlmModel>, draft: &LlmModelDraft) -> CommandResult<()> {
    match existing {
        Some(model)
            if model.has_api_key
                && draft.api_key.is_none()
                && !same_origin(&model.base_url, &draft.base_url) =>
        {
            Err(CoreError::InvalidConfig(vec![FieldError::new(
                "apiKey",
                "换了接口地址的域名：请重新填写 API Key（原来的 Key 不会发给新地址）。",
            )])
            .into())
        }
        _ => Ok(()),
    }
}

pub async fn list_llm_models(state: &AppState) -> CommandResult<LlmSettings> {
    Ok(state.llm.load()?)
}

/// 先校验,再改密码存储,最后写配置。
pub async fn save_llm_model(state: &AppState, draft: LlmModelDraft) -> CommandResult<LlmModel> {
    let _guard = state.llm_lock.lock().await;
    let settings = state.llm.load()?;
    let existing = match &draft.id {
        Some(id) => Some(
            settings
                .get(id)
                .cloned()
                .ok_or_else(|| CoreError::NotFound(format!("找不到模型：{id}")))?,
        ),
        None => None,
    };
    let id = existing
        .as_ref()
        .map_or_else(|| uuid::Uuid::new_v4().to_string(), |m| m.id.clone());
    check_key_origin(existing.as_ref(), &draft)?;
    let key = draft.api_key.clone();
    let has_key = match key.as_deref() {
        None => existing.as_ref().is_some_and(|m| m.has_api_key),
        Some(key) => !key.trim().is_empty(),
    };
    let model = draft.into_model(id.clone(), has_key)?;
    match key.as_deref().map(str::trim) {
        None => {}
        Some("") => {
            if existing.as_ref().is_some_and(|m| m.has_api_key) {
                state.secrets.delete(&api_key_slot(&id))?;
            }
        }
        Some(key) => state.secrets.set(&api_key_slot(&id), key)?,
    }
    state.llm.save(&settings.upserted(model.clone()))?;
    Ok(model)
}

pub async fn delete_llm_model(state: &AppState, id: &str) -> CommandResult<LlmSettings> {
    let _guard = state.llm_lock.lock().await;
    let settings = state.llm.load()?;
    let model = settings
        .get(id)
        .cloned()
        .ok_or_else(|| CoreError::NotFound(format!("找不到模型：{id}")))?;
    let next = settings.removed(id);
    state.llm.save(&next)?;
    // 和删连接一样:不弹授权框,删不掉就留着。
    if model.has_api_key {
        match state.secrets.delete_quietly(&api_key_slot(id)) {
            Ok(QuietDelete::Removed) => {}
            Ok(QuietDelete::NeedsAuthorization) => {
                log::warn!("模型 {id} 已删除，它的 API Key 要授权才能删，留在了密码存储里")
            }
            Err(error) => log::warn!("模型 {id} 已删除，但清理密码存储失败：{error}"),
        }
    }
    Ok(next)
}

pub async fn set_active_llm_model(state: &AppState, id: &str) -> CommandResult<LlmSettings> {
    let _guard = state.llm_lock.lock().await;
    let next = state.llm.load()?.with_active(id)?;
    state.llm.save(&next)?;
    Ok(next)
}

/// 表单里的 Key:没填时用密码存储里已经存的(编辑已有模型、地址的主机没变时)。
fn key_for(state: &AppState, draft: &LlmModelDraft) -> CommandResult<Option<String>> {
    match draft.api_key.as_deref().map(str::trim) {
        Some("") => Ok(None),
        Some(key) => Ok(Some(key.to_owned())),
        None => match &draft.id {
            Some(id) => {
                let settings = state.llm.load()?;
                let saved = settings.get(id);
                check_key_origin(saved, draft)?;
                // 没存过 Key 的模型不去读密码存储,免得白弹一次授权框。
                if saved.is_some_and(|m| m.has_api_key) {
                    Ok(state.secrets.get(&api_key_slot(id))?)
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        },
    }
}

/// 「测试连接」:用表单里的设置(还没保存也行)发一句很短的话。
pub async fn test_llm_model(state: &AppState, draft: LlmModelDraft) -> CommandResult<TestReply> {
    let key = key_for(state, &draft)?;
    let id = draft.id.clone().unwrap_or_else(|| "draft".into());
    let model = draft.into_model(id, key.is_some())?;
    Ok(test_model(shared_client()?, &model, key.as_deref()).await?)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOutcome {
    /// 回答一共多少字。
    pub chars: usize,
    /// 完整的回答。事件和命令的返回不保证先后,前端以它为准,免得丢了最后几段。
    pub text: String,
}

/// 流式对话:每一段文字交给 `on_text`(推给前端),返回时回答已经完整。
pub async fn llm_chat(
    state: &AppState,
    request_id: &str,
    request: ChatRequest,
    mut on_text: impl FnMut(&str) + Send + 'static,
) -> CommandResult<ChatOutcome> {
    // 先登记:读配置、读密码存储(可能弹授权框)期间点的「停止」也算数。
    let _registered = state.chats.register(request_id)?;
    let model = state
        .llm
        .load()?
        .get(&request.model_id)
        .cloned()
        .ok_or_else(|| {
            CoreError::NotFound("找不到这个模型，可能已经删掉了。请在助手里重新选一个。".into())
        })?;
    let key = if model.has_api_key {
        state.secrets.get(&api_key_slot(&model.id))?
    } else {
        None
    };
    let client = shared_client()?;
    let task = tokio::spawn(async move {
        let mut text = String::new();
        let chars = stream_chat(
            client,
            &model,
            key.as_deref(),
            &request.system,
            &request.messages,
            &mut |piece| {
                text.push_str(piece);
                on_text(piece);
            },
        )
        .await?;
        Ok::<_, CoreError>((chars, text))
    });
    if !state.chats.attach(request_id, task.abort_handle()) {
        task.abort();
    }
    match task.await {
        Ok(Ok((chars, text))) => Ok(ChatOutcome { chars, text }),
        Ok(Err(error)) => Err(error.into()),
        Err(error) if error.is_cancelled() => Err(CoreError::Cancelled.into()),
        Err(error) => Err(CoreError::Llm(format!("对话任务异常结束：{error}")).into()),
    }
}

/// 已经结束(或没有这个对话)时返回 false。
pub fn cancel_llm_chat(state: &AppState, request_id: &str) -> bool {
    state.chats.cancel(request_id)
}

pub async fn schema_outline(
    state: &AppState,
    id: &str,
    database: &str,
    schema: Option<&str>,
) -> CommandResult<Vec<OutlineTable>> {
    let session = state.sessions.get(id).await?;
    Ok(session.schema_outline(database, schema).await?)
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
