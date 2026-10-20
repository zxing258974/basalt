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
    /// 导入时提交的表单。带着明文密码的才标为「保存密码」并写进钥匙串;没有密码的不标,
    /// 打开前就会弹密码框,而不是先拿空密码登录失败一次(会计入服务器的登录失败锁定)。
    /// 不支持的连接返回 `None`。
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
            save_password: password.is_some(),
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
    /// 和已有连接是同一个库(类型、地址、用户都相同),来源带着密码而已有连接没存密码:
    /// 导入时不新建连接,只把密码补到已有连接上。默认勾选。
    #[serde(rename_all = "camelCase")]
    FillPassword {
        existing_id: String,
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
                    Some((found, same_address)) => {
                        duplicate_status(source, endpoint, found, same_address)
                    }
                    None => CandidateStatus::New,
                };
                known.push(Known {
                    id: None,
                    name: source.name.trim(),
                    driver: *driver,
                    endpoint,
                    has_saved_password: false,
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
    /// 已保存的连接才有 id;本次来源里的重复项没有,只能跳过、不能补密码。
    id: Option<&'a str>,
    name: &'a str,
    driver: DriverKind,
    endpoint: &'a Endpoint,
    has_saved_password: bool,
}

impl<'a> From<&'a ConnectionConfig> for Known<'a> {
    fn from(config: &'a ConnectionConfig) -> Self {
        Self {
            id: Some(&config.id),
            name: &config.name,
            driver: config.driver,
            endpoint: &config.endpoint,
            has_saved_password: config.save_password,
        }
    }
}

/// 连接名相同,或者数据库类型和「连到哪、以谁的身份」都相同,就算重复。
/// 同时返回是否是后一种情况:只有地址也对得上,才允许把密码补过去。
fn find_duplicate<'a, 'k>(
    name: &str,
    driver: DriverKind,
    endpoint: &Endpoint,
    known: &'k [Known<'a>],
) -> Option<(&'k Known<'a>, bool)> {
    let name = name.trim();
    known.iter().find_map(|k| {
        let same = k.driver == driver && same_address(k.endpoint, endpoint);
        (same || k.name == name).then_some((k, same))
    })
}

/// 重复项是「补密码」还是单纯「已存在」。补密码要求地址一致、是网络连接、
/// 来源带明文密码、已有连接还没存密码;只靠名字对上的绝不补,免得把别的库的密码填进来。
fn duplicate_status(
    source: &SourceConnection,
    endpoint: &Endpoint,
    found: &Known<'_>,
    same_address: bool,
) -> CandidateStatus {
    let fillable = same_address
        && matches!(endpoint, Endpoint::Network { .. })
        && matches!(source.password, SourcePassword::Plain(_))
        && !found.has_saved_password;
    match (fillable, found.id) {
        (true, Some(id)) => CandidateStatus::FillPassword {
            existing_id: id.to_owned(),
            existing_name: found.name.to_owned(),
        },
        _ => CandidateStatus::Existing {
            existing_name: found.name.to_owned(),
        },
    }
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
