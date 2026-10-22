//! SQL 助手的模型配置:配置目录下的 `llm-models.json`,不含 API Key(Key 在密码存储里,见 [`api_key_slot`])。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Protocol;
use crate::error::{CoreError, CoreResult, FieldError};

/// 一个配置好的模型。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmModel {
    pub id: String,
    /// 给人看的名字,如「DeepSeek」「公司网关 Claude」。
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    /// 请求里的模型名,如 `gpt-4.1`、`claude-sonnet-4-5`。
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// 密码存储里存了 API Key。记在这里,列模型时不必去读密码存储(读别的版本存的条目会弹授权框)。
    #[serde(default)]
    pub has_api_key: bool,
}

/// 新建或编辑模型时前端交上来的表单。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelDraft {
    /// 为空表示新建。
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// `None`:不改密码存储里的;`Some("")`:删掉;其余:换成这个。
    #[serde(default)]
    pub api_key: Option<String>,
}

impl LlmModelDraft {
    /// 校验并转成要保存的模型。`has_api_key` 是保存之后密码存储里有没有 Key。
    pub fn into_model(self, id: String, has_api_key: bool) -> CoreResult<LlmModel> {
        let mut errors = Vec::new();
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            errors.push(FieldError::new("name", "起个名字，方便在助手里选。"));
        }
        let base_url = self.base_url.trim().to_owned();
        match reqwest::Url::parse(&base_url) {
            Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => {}
            _ => errors.push(FieldError::new(
                "baseUrl",
                "要以 http:// 或 https:// 开头，如 https://api.openai.com/v1。",
            )),
        }
        let model = self.model.trim().to_owned();
        if model.is_empty() {
            errors.push(FieldError::new("model", "填写接口里的模型名称。"));
        }
        if self.max_tokens.is_some_and(|n| n == 0 || n > 1_000_000) {
            errors.push(FieldError::new(
                "maxTokens",
                "在 1 到 1000000 之间，不填用默认值。",
            ));
        }
        if self
            .temperature
            .is_some_and(|t| !(0.0..=2.0).contains(&t) || t.is_nan())
        {
            errors.push(FieldError::new(
                "temperature",
                "在 0 到 2 之间，不填用模型的默认值。",
            ));
        }
        if !errors.is_empty() {
            return Err(CoreError::InvalidConfig(errors));
        }
        Ok(LlmModel {
            id,
            name,
            protocol: self.protocol,
            base_url,
            model,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            has_api_key,
        })
    }
}

/// 已存的 API Key 能不能用在 `base_url` 上:协议、域名、端口都没变才行。
/// 编辑时换了主机却没重填 Key,原来的 Key 不能发到新地址去。
pub fn same_origin(saved: &str, base_url: &str) -> bool {
    let origin = |url: &str| {
        reqwest::Url::parse(url.trim())
            .ok()
            .map(|u| u.origin().ascii_serialization())
    };
    matches!((origin(saved), origin(base_url)), (Some(a), Some(b)) if a == b)
}

/// 模型 API Key 在密码存储里的账户名。
pub fn api_key_slot(model_id: &str) -> String {
    format!("llm:{model_id}:api_key")
}

/// 全部模型和助手默认用哪一个。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettings {
    #[serde(default)]
    pub models: Vec<LlmModel>,
    #[serde(default)]
    pub active: Option<String>,
}

impl LlmSettings {
    pub fn get(&self, id: &str) -> Option<&LlmModel> {
        self.models.iter().find(|m| m.id == id)
    }

    /// 新的放最后,已有的原地替换。还没有默认模型时,第一个保存的就是默认的。
    pub fn upserted(&self, model: LlmModel) -> Self {
        let mut models = self.models.clone();
        match models.iter_mut().find(|m| m.id == model.id) {
            Some(slot) => *slot = model.clone(),
            None => models.push(model.clone()),
        }
        let active = self.active.clone().or(Some(model.id));
        Self { models, active }
    }

    /// 删掉的是默认模型时,改用剩下的第一个。
    pub fn removed(&self, id: &str) -> Self {
        let models: Vec<LlmModel> = self.models.iter().filter(|m| m.id != id).cloned().collect();
        let active = match &self.active {
            Some(active) if active != id => Some(active.clone()),
            _ => models.first().map(|m| m.id.clone()),
        };
        Self { models, active }
    }

    pub fn with_active(&self, id: &str) -> CoreResult<Self> {
        if self.get(id).is_none() {
            return Err(CoreError::NotFound(format!("找不到模型：{id}")));
        }
        Ok(Self {
            models: self.models.clone(),
            active: Some(id.to_owned()),
        })
    }
}

/// `llm-models.json` 的读写:文件不存在算空,内容坏了报错不覆盖,先写临时文件再改名。
#[derive(Clone, Debug)]
pub struct LlmRepository {
    path: PathBuf,
}

impl LlmRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> CoreResult<LlmSettings> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| storage_error(&self.path, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LlmSettings::default()),
            Err(e) => Err(storage_error(&self.path, e)),
        }
    }

    pub fn save(&self, settings: &LlmSettings) -> CoreResult<()> {
        let dir = self.path.parent().ok_or_else(|| {
            CoreError::Storage(format!("无效的配置路径：{}", self.path.display()))
        })?;
        std::fs::create_dir_all(dir).map_err(|e| storage_error(dir, e))?;
        let json =
            serde_json::to_string_pretty(settings).map_err(|e| storage_error(&self.path, e))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| storage_error(&tmp, e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| storage_error(&self.path, e))
    }
}

fn storage_error(path: &Path, error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("{}：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> LlmModelDraft {
        LlmModelDraft {
            id: None,
            name: " DeepSeek ".into(),
            protocol: Protocol::Openai,
            base_url: " https://api.deepseek.com/v1 ".into(),
            model: "deepseek-chat".into(),
            max_tokens: None,
            temperature: Some(0.2),
            api_key: Some("sk-1".into()),
        }
    }

    #[test]
    fn drafts_are_trimmed_and_checked_field_by_field() {
        let model = draft().into_model("m1".into(), true).unwrap();
        assert_eq!(
            (model.name.as_str(), model.base_url.as_str()),
            ("DeepSeek", "https://api.deepseek.com/v1")
        );
        assert!(model.has_api_key);

        let bad = LlmModelDraft {
            name: " ".into(),
            base_url: "api.deepseek.com".into(),
            model: "".into(),
            max_tokens: Some(0),
            temperature: Some(3.0),
            ..draft()
        };
        let CoreError::InvalidConfig(errors) = bad.into_model("m1".into(), false).unwrap_err()
        else {
            panic!("应该是字段错误");
        };
        let fields: Vec<&str> = errors.iter().map(|e| e.field.as_str()).collect();
        assert_eq!(
            fields,
            ["name", "baseUrl", "model", "maxTokens", "temperature"]
        );
        let ftp = LlmModelDraft {
            base_url: "ftp://host/v1".into(),
            ..draft()
        };
        assert!(ftp.into_model("m1".into(), false).is_err());
    }

    #[test]
    fn keys_only_follow_the_same_origin() {
        assert!(same_origin(
            "https://api.openai.com/v1",
            "https://api.openai.com/v2/"
        ));
        assert!(!same_origin(
            "https://api.openai.com/v1",
            "https://gw.example.com/v1"
        ));
        assert!(!same_origin(
            "https://api.openai.com",
            "http://api.openai.com"
        ));
        assert!(!same_origin(
            "http://127.0.0.1:11434",
            "http://127.0.0.1:8080"
        ));
        assert!(!same_origin("https://api.openai.com", "not a url"));
    }

    #[test]
    fn settings_keep_a_sensible_active_model() {
        let a = draft().into_model("a".into(), true).unwrap();
        let b = LlmModel {
            id: "b".into(),
            ..a.clone()
        };
        let settings = LlmSettings::default()
            .upserted(a.clone())
            .upserted(b.clone());
        assert_eq!(settings.active.as_deref(), Some("a"), "第一个保存的是默认");
        let renamed = LlmModel {
            name: "改名".into(),
            ..a.clone()
        };
        let settings = settings.upserted(renamed);
        assert_eq!(settings.models[0].name, "改名");
        assert_eq!(settings.models.len(), 2);
        let settings = settings.with_active("b").unwrap();
        assert_eq!(settings.removed("a").active.as_deref(), Some("b"));
        assert_eq!(
            settings.removed("b").active.as_deref(),
            Some("a"),
            "删掉默认的,换成剩下的"
        );
        assert_eq!(settings.removed("b").removed("a").active, None);
        assert!(settings.with_active("x").is_err());
        assert_eq!(api_key_slot("a"), "llm:a:api_key");
    }

    #[test]
    fn repository_round_trips_and_never_overwrites_a_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = LlmRepository::new(dir.path().join("llm-models.json"));
        assert_eq!(repo.load().unwrap(), LlmSettings::default());
        let settings =
            LlmSettings::default().upserted(draft().into_model("a".into(), true).unwrap());
        repo.save(&settings).unwrap();
        assert_eq!(repo.load().unwrap(), settings);
        let saved = std::fs::read_to_string(dir.path().join("llm-models.json")).unwrap();
        assert!(!saved.contains("sk-1"), "API Key 不写进文件");

        std::fs::write(dir.path().join("llm-models.json"), "{坏了").unwrap();
        assert_eq!(repo.load().unwrap_err().code(), "storage");
    }
}
