//! Navicat 的连接,两个来源:
//! - 本机 Navicat 16 的 `conn.plist`:有地址、用户名,没有密码(密码在只有 Navicat 能读的钥匙串里,不去碰);
//! - 「文件 → 导出连接」生成的 `.ncx`:勾了「导出密码」时带 AES 加密的密码。
//!
//! 两边先解析成 [`RawConnection`],再统一映射成 [`SourceConnection`]。

mod cipher;
mod conn_plist;
mod ncx;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use cipher::decrypt_password;
pub use conn_plist::{parse_conn_plist, parse_vgroups};
pub use ncx::parse_ncx;

use super::{SourceConnection, SourcePassword, SourceTarget};
use crate::config::{self, ConnectionDraft, DriverKind, Endpoint};
use crate::error::{CoreError, CoreResult};

pub const TUNNEL_SSL_WARNING: &str = "隧道 / SSL 设置未迁移（M4 支持）";
const SUPPORTED_LIST: &str = "MySQL、MariaDB、PostgreSQL 和 SQLite";
/// 连接文件不会有这么大,超过就当选错了文件,不整个读进内存。
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

/// 本机 Navicat 配置的位置(相对用户主目录)。
const CONN_PLIST: &str =
    "Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist";
const VGROUP_JSON: &str = "Library/Application Support/PremiumSoft CyberTech/Navicat CC/Navicat Premium/Profiles/vgroup.json";

/// 两种来源共用的中间形态:字段都是来源里的原始文本。
#[derive(Debug, Default)]
struct RawConnection {
    name: String,
    conn_type: String,
    host: String,
    port: String,
    username: String,
    database: String,
    file_path: String,
    password: SourcePassword,
    uses_tunnel: bool,
    uses_ssl: bool,
}

/// Navicat 的类型名 → Basalt 的数据库类型;不支持的返回给用户看的类型名。
fn map_type(conn_type: &str) -> Result<DriverKind, String> {
    let key: String = conn_type
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase();
    match key.as_str() {
        "MYSQL" => Ok(DriverKind::Mysql),
        "MARIADB" => Ok(DriverKind::Mariadb),
        "POSTGRESQL" | "POSTGRES" | "PGSQL" => Ok(DriverKind::Postgres),
        "SQLITE" => Ok(DriverKind::Sqlite),
        "REDIS" => Err("Redis".into()),
        "ORACLE" => Err("Oracle".into()),
        "SQLSERVER" | "MSSQL" => Err("SQL Server".into()),
        "MONGODB" => Err("MongoDB".into()),
        "SNOWFLAKE" => Err("Snowflake".into()),
        "" => Err("未知类型".into()),
        _ => Err(conn_type.trim().to_owned()),
    }
}

fn display_type(conn_type: &str) -> String {
    match map_type(conn_type) {
        Ok(DriverKind::Mysql) => "MySQL".into(),
        Ok(DriverKind::Mariadb) => "MariaDB".into(),
        Ok(DriverKind::Postgres) => "PostgreSQL".into(),
        Ok(DriverKind::Sqlite) => "SQLite".into(),
        Err(label) => label,
    }
}

fn non_blank(value: &str) -> Option<String> {
    Some(value.trim().to_owned()).filter(|v| !v.is_empty())
}

/// 映射成连接方式,并用连接表单同一套校验把不完整的拦下来。返回目标和附加提示。
fn resolve_target(raw: &RawConnection) -> (SourceTarget, Vec<String>) {
    let driver = match map_type(&raw.conn_type) {
        Ok(driver) => driver,
        Err(label) => {
            let reason = format!("Basalt 暂不支持 {label}，目前只支持 {SUPPORTED_LIST}");
            return (SourceTarget::Unsupported(reason), vec![]);
        }
    };
    let mut warnings = Vec::new();
    let endpoint = if driver.is_file_based() {
        if raw.file_path.trim().is_empty() {
            let reason = "Navicat 里没有记录 SQLite 数据库文件的路径".to_owned();
            return (SourceTarget::Unsupported(reason), vec![]);
        }
        Endpoint::File {
            path: raw.file_path.trim().to_owned(),
        }
    } else {
        let default_port = driver.default_port().unwrap_or_default();
        let port = match raw.port.trim() {
            "" => default_port,
            text => text
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .unwrap_or_else(|| {
                    warnings.push(format!("端口「{text}」无效，已改用默认端口 {default_port}"));
                    default_port
                }),
        };
        Endpoint::Network {
            host: raw.host.trim().to_owned(),
            port,
            username: raw.username.trim().to_owned(),
            database: non_blank(&raw.database),
        }
    };

    let probe = ConnectionDraft {
        id: None,
        name: raw.name.clone(),
        group: None,
        driver,
        endpoint: endpoint.clone(),
        save_password: true,
        password: None,
    };
    let errors = config::validate(&probe);
    if !errors.is_empty() {
        let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
        let reason = format!("连接信息不完整：{}", messages.join("；"));
        return (SourceTarget::Unsupported(reason), vec![]);
    }
    if raw.uses_tunnel || raw.uses_ssl {
        warnings.push(TUNNEL_SSL_WARNING.to_owned());
    }
    (SourceTarget::Supported { driver, endpoint }, warnings)
}

/// 来源内唯一的 id:`类型:连接名`,已被占用就依次加 `#2`、`#3`……直到不重复
/// (连接名本身带 `#2` 也不会撞上)。
fn unique_id(raw: &RawConnection, used: &mut HashSet<String>) -> String {
    let base = format!("{}:{}", raw.conn_type.trim(), raw.name);
    let id = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}#{n}")))
        .find(|candidate| !used.contains(candidate))
        .unwrap_or(base);
    used.insert(id.clone());
    id
}

/// 映射一批原始连接,并分配来源内唯一的 id。
fn into_sources(raws: Vec<RawConnection>) -> Vec<SourceConnection> {
    let mut used = HashSet::new();
    raws.into_iter()
        .map(|raw| {
            let id = unique_id(&raw, &mut used);
            let (target, warnings) = resolve_target(&raw);
            SourceConnection {
                id,
                name: raw.name.trim().to_owned(),
                group: None,
                source_type: display_type(&raw.conn_type),
                target,
                password: raw.password,
                warnings,
            }
        })
        .collect()
}

fn read_source(path: &Path) -> CoreResult<Vec<u8>> {
    let error = |e: std::io::Error| CoreError::Storage(format!("{}：{e}", path.display()));
    let size = std::fs::metadata(path).map_err(error)?.len();
    if size > MAX_SOURCE_BYTES {
        return Err(CoreError::Unsupported(format!(
            "{} 太大，不像是 Navicat 的连接文件",
            path.display()
        )));
    }
    std::fs::read(path).map_err(error)
}

/// 本机 Navicat 的配置文件位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavicatLocation {
    pub conn_plist: PathBuf,
    pub vgroup_json: PathBuf,
}

impl NavicatLocation {
    pub fn under_home(home: &Path) -> Self {
        Self {
            conn_plist: home.join(CONN_PLIST),
            vgroup_json: home.join(VGROUP_JSON),
        }
    }

    /// 读本机 Navicat 的连接。分组文件读不到或认不出时当作没有分组。
    pub fn load(&self) -> CoreResult<Vec<SourceConnection>> {
        if !self.conn_plist.exists() {
            return Err(CoreError::Storage(format!(
                "没有找到本机 Navicat 的连接配置：{}",
                self.conn_plist.display()
            )));
        }
        let sources = parse_conn_plist(&read_source(&self.conn_plist)?)?;
        let groups = std::fs::read_to_string(&self.vgroup_json)
            .map(|json| parse_vgroups(&json))
            .unwrap_or_default();
        Ok(sources
            .into_iter()
            .map(|source| SourceConnection {
                group: groups.get(&source.name).cloned(),
                ..source
            })
            .collect())
    }
}

/// 读 Navicat「导出连接」生成的 .ncx 文件。
pub fn load_ncx(path: &Path) -> CoreResult<Vec<SourceConnection>> {
    let bytes = read_source(path)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        CoreError::Unsupported(format!(
            "{} 不是 UTF-8 文本，不像是 .ncx 文件",
            path.display()
        ))
    })?;
    parse_ncx(&text)
}

#[cfg(test)]
mod tests;
