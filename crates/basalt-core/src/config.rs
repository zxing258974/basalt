//! 连接配置:模型、校验、不可变的连接列表,以及落盘到 JSON 的仓库。
//! 密码不在这里——见 [`crate::secret`]。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, FieldError};

const MAX_NAME_CHARS: usize = 100;
const STORE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Mysql,
    Mariadb,
    Postgres,
    Sqlite,
}

impl DriverKind {
    pub fn default_port(self) -> Option<u16> {
        match self {
            Self::Mysql | Self::Mariadb => Some(3306),
            Self::Postgres => Some(5432),
            Self::Sqlite => None,
        }
    }

    pub fn is_file_based(self) -> bool {
        matches!(self, Self::Sqlite)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Endpoint {
    #[serde(rename_all = "camelCase")]
    Network {
        host: String,
        port: u16,
        username: String,
        #[serde(default)]
        database: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    File { path: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group: Option<String>,
    pub driver: DriverKind,
    pub endpoint: Endpoint,
    #[serde(default)]
    pub save_password: bool,
}

/// 连接表单提交的内容。`id` 为空表示新建。`Debug` 不输出密码。
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDraft {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub group: Option<String>,
    pub driver: DriverKind,
    pub endpoint: Endpoint,
    #[serde(default)]
    pub save_password: bool,
    /// `None` 表示沿用已保存的密码;`Some("")` 表示空密码。
    #[serde(default)]
    pub password: Option<String>,
}

impl std::fmt::Debug for ConnectionDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionDraft")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("group", &self.group)
            .field("driver", &self.driver)
            .field("endpoint", &self.endpoint)
            .field("save_password", &self.save_password)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

impl ConnectionDraft {
    /// 校验并整理(去首尾空白、空分组归一为 None)成配置。
    pub fn into_config(self, id: String) -> CoreResult<ConnectionConfig> {
        let errors = validate(&self);
        if !errors.is_empty() {
            return Err(CoreError::InvalidConfig(errors));
        }
        let endpoint = match self.endpoint {
            Endpoint::Network {
                host,
                port,
                username,
                database,
            } => Endpoint::Network {
                host: host.trim().to_owned(),
                port,
                username: username.trim().to_owned(),
                database: non_blank(database),
            },
            Endpoint::File { path } => Endpoint::File {
                path: path.trim().to_owned(),
            },
        };
        Ok(ConnectionConfig {
            id,
            name: self.name.trim().to_owned(),
            group: non_blank(self.group),
            driver: self.driver,
            endpoint,
            save_password: self.save_password,
        })
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// 校验表单,返回全部字段错误而不是遇到第一个就停。
pub fn validate(draft: &ConnectionDraft) -> Vec<FieldError> {
    let mut errors = Vec::new();
    let name = draft.name.trim();
    if name.is_empty() {
        errors.push(FieldError::new("name", "请填写连接名"));
    } else if name.chars().count() > MAX_NAME_CHARS {
        errors.push(FieldError::new("name", "连接名不能超过 100 个字符"));
    }

    match (&draft.endpoint, draft.driver.is_file_based()) {
        (
            Endpoint::Network {
                host,
                port,
                username,
                ..
            },
            false,
        ) => {
            let host = host.trim();
            if host.is_empty() {
                errors.push(FieldError::new("host", "请填写主机地址"));
            } else if host.chars().any(char::is_whitespace) {
                errors.push(FieldError::new("host", "主机地址不能包含空格"));
            }
            if *port == 0 {
                errors.push(FieldError::new("port", "端口范围是 1–65535"));
            }
            if username.trim().is_empty() {
                errors.push(FieldError::new("username", "请填写用户名"));
            }
        }
        (Endpoint::File { path }, true) => {
            let path = path.trim();
            if path.is_empty() {
                errors.push(FieldError::new("path", "请选择数据库文件"));
            } else if !Path::new(path).is_absolute() {
                errors.push(FieldError::new("path", "请使用绝对路径"));
            }
        }
        _ => errors.push(FieldError::new("endpoint", "连接方式与数据库类型不匹配")),
    }
    errors
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionList {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub connections: Vec<ConnectionConfig>,
}

impl ConnectionList {
    pub fn get(&self, id: &str) -> Option<&ConnectionConfig> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// 返回替换(同 id,保持原位置)或追加后的新列表。
    pub fn upserted(&self, config: ConnectionConfig) -> Self {
        let connections = if self.get(&config.id).is_some() {
            self.connections
                .iter()
                .map(|c| {
                    if c.id == config.id {
                        config.clone()
                    } else {
                        c.clone()
                    }
                })
                .collect()
        } else {
            self.connections.iter().cloned().chain([config]).collect()
        };
        Self::with_connections(connections)
    }

    pub fn removed(&self, id: &str) -> Self {
        let connections = self
            .connections
            .iter()
            .filter(|c| c.id != id)
            .cloned()
            .collect();
        Self::with_connections(connections)
    }

    /// 复制一个连接,名称加「副本」后缀,紧跟在原连接后面。
    pub fn duplicated(&self, id: &str, new_id: String) -> Option<(Self, ConnectionConfig)> {
        let original = self.get(id)?;
        let copy = ConnectionConfig {
            id: new_id,
            name: format!("{} 副本", original.name),
            ..original.clone()
        };
        let connections = self
            .connections
            .iter()
            .flat_map(|c| {
                let extra = (c.id == id).then(|| copy.clone());
                std::iter::once(c.clone()).chain(extra)
            })
            .collect();
        Some((Self::with_connections(connections), copy))
    }

    fn with_connections(connections: Vec<ConnectionConfig>) -> Self {
        Self {
            version: STORE_VERSION,
            connections,
        }
    }
}

/// 连接列表的 JSON 文件仓库。
#[derive(Clone, Debug)]
pub struct ConnectionRepository {
    path: PathBuf,
}

impl ConnectionRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 文件不存在视为空列表;内容损坏则报错,绝不覆盖。
    pub fn load(&self) -> CoreResult<ConnectionList> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConnectionList::default());
            }
            Err(e) => return Err(storage_error(&self.path, e)),
        };
        serde_json::from_str(&raw).map_err(|e| storage_error(&self.path, e))
    }

    /// 先写同目录临时文件再改名,写到一半崩溃也不会留下半个文件。
    pub fn save(&self, list: &ConnectionList) -> CoreResult<()> {
        let dir = self.path.parent().ok_or_else(|| {
            CoreError::Storage(format!("无效的配置路径：{}", self.path.display()))
        })?;
        std::fs::create_dir_all(dir).map_err(|e| storage_error(dir, e))?;

        let stored = ConnectionList::with_connections(list.connections.clone());
        let json =
            serde_json::to_string_pretty(&stored).map_err(|e| storage_error(&self.path, e))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| storage_error(&tmp, e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| storage_error(&self.path, e))
    }
}

fn storage_error(path: &Path, error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("{}:{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_draft() -> ConnectionDraft {
        ConnectionDraft {
            id: None,
            name: "  本地 MySQL  ".into(),
            group: Some("  ".into()),
            driver: DriverKind::Mysql,
            endpoint: Endpoint::Network {
                host: " 127.0.0.1 ".into(),
                port: 3306,
                username: " root ".into(),
                database: Some("".into()),
            },
            save_password: true,
            password: Some("secret".into()),
        }
    }

    fn sqlite_draft(path: &str) -> ConnectionDraft {
        ConnectionDraft {
            id: None,
            name: "笔记".into(),
            group: None,
            driver: DriverKind::Sqlite,
            endpoint: Endpoint::File { path: path.into() },
            save_password: false,
            password: None,
        }
    }

    fn config(id: &str, name: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.into(),
            name: name.into(),
            group: None,
            driver: DriverKind::Postgres,
            endpoint: Endpoint::Network {
                host: "db".into(),
                port: 5432,
                username: "app".into(),
                database: None,
            },
            save_password: false,
        }
    }

    fn fields(errors: &[FieldError]) -> Vec<&str> {
        errors.iter().map(|e| e.field.as_str()).collect()
    }

    #[test]
    fn draft_debug_output_redacts_password() {
        let debug = format!("{:?}", network_draft());
        assert!(!debug.contains("secret"), "{debug}");
        assert!(debug.contains("***"));
    }

    #[test]
    fn default_ports_follow_each_driver() {
        assert_eq!(DriverKind::Mysql.default_port(), Some(3306));
        assert_eq!(DriverKind::Mariadb.default_port(), Some(3306));
        assert_eq!(DriverKind::Postgres.default_port(), Some(5432));
        assert_eq!(DriverKind::Sqlite.default_port(), None);
        assert!(DriverKind::Sqlite.is_file_based());
        assert!(!DriverKind::Postgres.is_file_based());
    }

    #[test]
    fn valid_network_draft_has_no_errors() {
        assert!(validate(&network_draft()).is_empty());
    }

    #[test]
    fn reports_every_missing_network_field_at_once() {
        let draft = ConnectionDraft {
            name: " ".into(),
            endpoint: Endpoint::Network {
                host: "".into(),
                port: 0,
                username: "".into(),
                database: None,
            },
            ..network_draft()
        };
        assert_eq!(
            fields(&validate(&draft)),
            ["name", "host", "port", "username"]
        );
    }

    #[test]
    fn rejects_host_with_whitespace_inside() {
        let draft = ConnectionDraft {
            endpoint: Endpoint::Network {
                host: "my host".into(),
                port: 3306,
                username: "root".into(),
                database: None,
            },
            ..network_draft()
        };
        assert_eq!(fields(&validate(&draft)), ["host"]);
    }

    #[test]
    fn rejects_overlong_name() {
        let draft = ConnectionDraft {
            name: "库".repeat(MAX_NAME_CHARS + 1),
            ..network_draft()
        };
        assert_eq!(fields(&validate(&draft)), ["name"]);
    }

    /// 两个平台上都是绝对路径(Windows 的绝对路径要带盘符,/tmp 那种不算)。
    fn absolute(name: &str) -> String {
        std::env::temp_dir().join(name).display().to_string()
    }

    #[test]
    fn sqlite_requires_absolute_file_path() {
        assert!(validate(&sqlite_draft(&absolute("notes.db"))).is_empty());
        assert_eq!(fields(&validate(&sqlite_draft("notes.db"))), ["path"]);
        assert_eq!(fields(&validate(&sqlite_draft(""))), ["path"]);
    }

    #[test]
    fn driver_and_endpoint_kind_must_match() {
        let sqlite_with_network = ConnectionDraft {
            driver: DriverKind::Sqlite,
            ..network_draft()
        };
        assert_eq!(fields(&validate(&sqlite_with_network)), ["endpoint"]);

        let mysql_with_file = ConnectionDraft {
            driver: DriverKind::Mysql,
            ..sqlite_draft(&absolute("a.db"))
        };
        assert_eq!(fields(&validate(&mysql_with_file)), ["endpoint"]);
    }

    #[test]
    fn into_config_trims_and_normalizes_blank_optionals() {
        let config = network_draft().into_config("c1".into()).unwrap();
        assert_eq!(config.id, "c1");
        assert_eq!(config.name, "本地 MySQL");
        assert_eq!(config.group, None);
        assert_eq!(
            config.endpoint,
            Endpoint::Network {
                host: "127.0.0.1".into(),
                port: 3306,
                username: "root".into(),
                database: None,
            }
        );
        assert!(config.save_password);
    }

    #[test]
    fn into_config_rejects_invalid_draft() {
        let draft = ConnectionDraft {
            name: "".into(),
            ..network_draft()
        };
        let error = draft.into_config("c1".into()).unwrap_err();
        assert_eq!(error.code(), "invalid_config");
        assert_eq!(fields(error.field_errors()), ["name"]);
    }

    #[test]
    fn upserted_appends_new_and_replaces_existing_in_place() {
        let list = ConnectionList::default()
            .upserted(config("a", "A"))
            .upserted(config("b", "B"));
        let updated = list.upserted(config("a", "A2"));

        assert_eq!(list.get("a").unwrap().name, "A", "原列表不应被修改");
        let names: Vec<_> = updated
            .connections
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["A2", "B"]);
    }

    #[test]
    fn removed_drops_only_the_matching_id() {
        let list = ConnectionList::default()
            .upserted(config("a", "A"))
            .upserted(config("b", "B"));
        let after = list.removed("a");
        assert_eq!(list.connections.len(), 2);
        assert!(after.get("a").is_none());
        assert!(after.get("b").is_some());
        assert_eq!(after.removed("missing"), after);
    }

    #[test]
    fn duplicated_inserts_copy_right_after_original() {
        let list = ConnectionList::default()
            .upserted(config("a", "A"))
            .upserted(config("b", "B"));
        let (after, copy) = list.duplicated("a", "a2".into()).unwrap();
        assert_eq!(copy.id, "a2");
        assert_eq!(copy.name, "A 副本");
        let ids: Vec<_> = after.connections.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "a2", "b"]);
        assert!(list.duplicated("missing", "x".into()).is_none());
    }

    #[test]
    fn repository_round_trips_and_treats_missing_file_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let repo = ConnectionRepository::new(dir.path().join("nested/connections.json"));

        assert_eq!(repo.load().unwrap(), ConnectionList::default());

        let list = ConnectionList::default().upserted(config("a", "A"));
        repo.save(&list).unwrap();
        let loaded = repo.load().unwrap();
        assert_eq!(loaded.connections, list.connections);
        assert_eq!(loaded.version, STORE_VERSION);
    }

    #[test]
    fn repository_refuses_corrupt_file_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, "{ not json").unwrap();
        let repo = ConnectionRepository::new(&path);

        let error = repo.load().unwrap_err();
        assert_eq!(error.code(), "storage");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn saved_file_never_contains_password_field() {
        let dir = tempfile::tempdir().unwrap();
        let repo = ConnectionRepository::new(dir.path().join("connections.json"));
        let config = network_draft().into_config("c1".into()).unwrap();
        repo.save(&ConnectionList::default().upserted(config))
            .unwrap();
        let raw = std::fs::read_to_string(repo.path()).unwrap();
        assert!(!raw.contains("secret"));
        assert!(!raw.contains("password\""));
    }
}
