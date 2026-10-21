//! Navicat 的连接,两个来源:
//! - 本机 Navicat 16 的 `conn.plist`:有地址、用户名,没有密码(密码在只有 Navicat 能读的钥匙串里,不去碰);
//! - 「文件 → 导出连接」生成的 `.ncx`:勾了「导出密码」时带 AES 加密的密码。
//!
//! 两边先解析成 [`RawConnection`],再统一映射成 [`SourceConnection`]。

mod cipher;
mod conn_plist;
mod ncx;
pub mod queries;
mod transport;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use cipher::decrypt_password;
pub use conn_plist::{parse_conn_plist, parse_vgroups};
pub use ncx::parse_ncx;

use super::{SourceConnection, SourcePassword, SourceTarget};
use crate::config::{self, ConnectionDraft, DriverKind, Endpoint};
use crate::error::{CoreError, CoreResult};
use transport::{RawSsh, RawSsl, Transport, map_transport};

/// HTTP 隧道是 Navicat 自己的一套(服务器上放它的 PHP 脚本),Basalt 不支持。
pub const HTTP_TUNNEL_WARNING: &str = "HTTP 隧道 Basalt 不支持，没有迁移；不经隧道可能连不上";

/// 读不了本机 Navicat 时的说明。macOS 上只会是找不到用户主目录;Windows 版 Navicat 把连接存在注册表里,
/// 格式没有核实过,先不读,请用户导出 .ncx。
pub fn local_unavailable() -> CoreError {
    if cfg!(target_os = "macos") {
        CoreError::Storage("找不到用户主目录，无法定位本机 Navicat".into())
    } else {
        CoreError::Unsupported(
            "Windows 版 Navicat 的连接和查询存在注册表和它自己的目录里，Basalt 还读不了。请在 Navicat 里「导出连接」成 .ncx（勾上「导出密码」），再从这里导入 .ncx 文件。".into(),
        )
    }
}
const SUPPORTED_LIST: &str = "MySQL、MariaDB、PostgreSQL 和 SQLite";
/// 连接文件不会有这么大,超过就当选错了文件,不整个读进内存。
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

/// 本机 Navicat 配置的位置(相对用户主目录)。
const CONN_PLIST: &str =
    "Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist";
const VGROUP_JSON: &str = "Library/Application Support/PremiumSoft CyberTech/Navicat CC/Navicat Premium/Profiles/vgroup.json";
/// 保存的查询所在的目录,见 [`queries`]。
const SETTINGS_DIR: &str =
    "Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/Settings";

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
    /// 用了 SSH 隧道时的设置。
    ssh: Option<RawSsh>,
    /// 开了 SSL 时的设置。
    ssl: Option<RawSsl>,
    /// 解析来源时就知道迁移不了的设置(HTTP 隧道、格式没核实过的字段……)。
    notes: Vec<String>,
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

/// 映射成连接方式(连同隧道和 SSL),并用连接表单同一套校验把不完整的拦下来。返回目标、隧道与 SSL、附加提示。
fn resolve_target(raw: &RawConnection) -> (SourceTarget, Transport, Vec<String>) {
    let unsupported = |reason: String| {
        (
            SourceTarget::Unsupported(reason),
            Transport::default(),
            vec![],
        )
    };
    let driver = match map_type(&raw.conn_type) {
        Ok(driver) => driver,
        Err(label) => {
            return unsupported(format!(
                "Basalt 暂不支持 {label}，目前只支持 {SUPPORTED_LIST}"
            ));
        }
    };
    let mut warnings = Vec::new();
    let endpoint = if driver.is_file_based() {
        if raw.file_path.trim().is_empty() {
            return unsupported("Navicat 里没有记录 SQLite 数据库文件的路径".to_owned());
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

    let transport = if driver.is_file_based() {
        Transport::default()
    } else {
        map_transport(driver, raw.ssh.as_ref(), raw.ssl.as_ref())
    };
    let probe = ConnectionDraft {
        id: None,
        name: raw.name.clone(),
        group: None,
        driver,
        endpoint: endpoint.clone(),
        save_password: true,
        password: None,
        ssh: transport.ssh.clone(),
        ssl: transport.ssl.clone(),
        ssh_secret: None,
    };
    let errors = config::validate(&probe);
    if !errors.is_empty() {
        let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
        return unsupported(format!("连接信息不完整：{}", messages.join("；")));
    }
    warnings.extend(raw.notes.iter().cloned());
    warnings.extend(transport.warnings.iter().cloned());
    (
        SourceTarget::Supported { driver, endpoint },
        transport,
        warnings,
    )
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
            let (target, transport, warnings) = resolve_target(&raw);
            SourceConnection {
                id,
                name: raw.name.trim().to_owned(),
                group: None,
                source_type: display_type(&raw.conn_type),
                target,
                password: raw.password,
                ssh: transport.ssh,
                ssh_secret: transport.ssh_secret,
                ssl: transport.ssl,
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
    pub settings: PathBuf,
}

impl NavicatLocation {
    pub fn under_home(home: &Path) -> Self {
        Self {
            conn_plist: home.join(CONN_PLIST),
            vgroup_json: home.join(VGROUP_JSON),
            settings: home.join(SETTINGS_DIR),
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
