//! 从别的客户端导入连接。各来源先解析成 [`SourceConnection`],再对照已有连接分类成预览候选
//! [`ImportCandidate`]。候选只描述「是什么、能不能导」,不带密码,可以放心交给前端。

pub mod navicat;

use serde::Serialize;

use crate::config::{ConnectionConfig, ConnectionDraft, ConnectionList, DriverKind, Endpoint};

/// 来源里的密码。`Debug` 不输出明文,也不实现 `Serialize`,不会经 IPC 传出去。
#[derive(Clone, Default, PartialEq, Eq)]
pub enum SourcePassword {
    /// 来源里没有密码,比如本机 Navicat 16 的 conn.plist。
    #[default]
    Absent,
    Plain(String),
    /// 有密文但解不开,比如更老版本 Navicat 用 Blowfish 加密的导出文件。
    Undecryptable,
}

impl std::fmt::Debug for SourcePassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent => f.write_str("Absent"),
            Self::Plain(_) => f.write_str("Plain(***)"),
            Self::Undecryptable => f.write_str("Undecryptable"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceTarget {
    Supported {
        driver: DriverKind,
        endpoint: Endpoint,
    },
    /// 附带给用户看的原因。
    Unsupported(String),
}

/// 来源里的一个连接,已经映射成 Basalt 的数据库类型与连接方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceConnection {
    /// 同一来源内唯一且稳定:扫描和导入各解析一次,靠它把勾选项对上。
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    /// 来源里的类型名,如 MySQL、Redis,原样展示给用户。
    pub source_type: String,
    pub target: SourceTarget,
    pub password: SourcePassword,
    /// 照常导入、但没有迁移过来的设置。
    pub warnings: Vec<String>,
}

impl SourceConnection {
    /// 导入时提交的表单。导入的连接一律「保存密码」:有明文就写进钥匙串,
    /// 没有就等第一次打开时由密码框补输。不支持的连接返回 `None`。
    pub fn to_draft(&self) -> Option<ConnectionDraft> {
        let SourceTarget::Supported { driver, endpoint } = &self.target else {
            return None;
        };
        let password = match &self.password {
            SourcePassword::Plain(password) => Some(password.clone()),
            SourcePassword::Absent | SourcePassword::Undecryptable => None,
        };
        Some(ConnectionDraft {
            id: None,
            name: self.name.clone(),
            group: self.group.clone(),
            driver: *driver,
            endpoint: endpoint.clone(),
            save_password: true,
            password,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PasswordStatus {
    Included,
    Absent,
    Undecryptable,
}

impl From<&SourcePassword> for PasswordStatus {
    fn from(password: &SourcePassword) -> Self {
        match password {
            SourcePassword::Absent => Self::Absent,
            SourcePassword::Plain(_) => Self::Included,
            SourcePassword::Undecryptable => Self::Undecryptable,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CandidateStatus {
    New,
    /// 和已有的连接重复,默认不勾选。
    #[serde(rename_all = "camelCase")]
    Existing {
        existing_name: String,
    },
    Unsupported {
        reason: String,
    },
}

/// 导入预览里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    pub source_type: String,
    pub driver: Option<DriverKind>,
    pub endpoint: Option<Endpoint>,
    pub status: CandidateStatus,
    pub password: PasswordStatus,
    pub warnings: Vec<String>,
}

/// 对照已有连接给每个来源连接定状态。同一来源里后出现的重复项也算「已存在」,
/// 免得默认勾选后导入两个一模一样的连接。
pub fn classify(sources: &[SourceConnection], existing: &ConnectionList) -> Vec<ImportCandidate> {
    let mut known: Vec<Known<'_>> = existing.connections.iter().map(Known::from).collect();
    let mut candidates = Vec::with_capacity(sources.len());
    for source in sources {
        let (driver, endpoint, status) = match &source.target {
            SourceTarget::Supported { driver, endpoint } => {
                let status = match find_duplicate(&source.name, *driver, endpoint, &known) {
                    Some(found) => CandidateStatus::Existing {
                        existing_name: found.name.to_owned(),
                    },
                    None => CandidateStatus::New,
                };
                known.push(Known {
                    name: source.name.trim(),
                    driver: *driver,
                    endpoint,
                });
                (Some(*driver), Some(endpoint.clone()), status)
            }
            SourceTarget::Unsupported(reason) => (
                None,
                None,
                CandidateStatus::Unsupported {
                    reason: reason.clone(),
                },
            ),
        };
        candidates.push(ImportCandidate {
            id: source.id.clone(),
            name: source.name.clone(),
            group: source.group.clone(),
            source_type: source.source_type.clone(),
            driver,
            endpoint,
            status,
            password: PasswordStatus::from(&source.password),
            warnings: source.warnings.clone(),
        });
    }
    candidates
}

/// 判重时用到的连接要素:已保存的连接和本次来源里排在前面的连接都算。
struct Known<'a> {
    name: &'a str,
    driver: DriverKind,
    endpoint: &'a Endpoint,
}

impl<'a> From<&'a ConnectionConfig> for Known<'a> {
    fn from(config: &'a ConnectionConfig) -> Self {
        Self {
            name: &config.name,
            driver: config.driver,
            endpoint: &config.endpoint,
        }
    }
}

/// 连接名相同,或者数据库类型和「连到哪、以谁的身份」都相同,就算重复。
fn find_duplicate<'a, 'k>(
    name: &str,
    driver: DriverKind,
    endpoint: &Endpoint,
    known: &'k [Known<'a>],
) -> Option<&'k Known<'a>> {
    let name = name.trim();
    known
        .iter()
        .find(|k| k.name == name || (k.driver == driver && same_address(k.endpoint, endpoint)))
}

fn same_address(a: &Endpoint, b: &Endpoint) -> bool {
    match (a, b) {
        (
            Endpoint::Network {
                host: host_a,
                port: port_a,
                username: user_a,
                ..
            },
            Endpoint::Network {
                host: host_b,
                port: port_b,
                username: user_b,
                ..
            },
        ) => {
            // 主机名不区分大小写;用户名在 MySQL / PostgreSQL 里都区分。
            host_a.trim().eq_ignore_ascii_case(host_b.trim())
                && port_a == port_b
                && user_a.trim() == user_b.trim()
        }
        (Endpoint::File { path: a }, Endpoint::File { path: b }) => a.trim() == b.trim(),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
