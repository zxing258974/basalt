//! Navicat 保存的查询。本机 Navicat 16 把它们存成普通的 .sql 文件:
//! `Settings/<项目>/<子项目>/<类型>/<连接名>/<库名>/<查询名>.sql`,PostgreSQL 多一层 schema。
//! 只读这些文件,不改 Navicat 的任何东西。
//!
//! 每条查询要放到 Basalt 的哪个连接下:先找同名(且类型相符)的连接;找不到,再按 conn.plist 里
//! 同名 Navicat 连接的地址找。都找不到或找到不止一个,就说明原因、不导入。

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{display_type, map_type, read_source};
use crate::config::{ConnectionConfig, DriverKind, Endpoint};
use crate::error::{CoreError, CoreResult};
use crate::import::{SourceConnection, SourceTarget};
use crate::query::saved::{SavedQueryRef, sanitize_name};

/// 一条 Navicat 查询文件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavicatQuery {
    /// 相对 Settings 目录的路径:来源内唯一且稳定,扫描和导入各读一次,靠它对上勾选项。
    pub id: String,
    /// 目录名里的类型,如 MySQL、PostgreSQL。
    pub conn_type: String,
    pub connection: String,
    pub database: String,
    pub schema: Option<String>,
    pub name: String,
    pub path: PathBuf,
}

fn visible_children(dir: &Path, want_dirs: bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut children: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| n.starts_with('.'));
            !hidden && path.is_dir() == want_dirs
        })
        .collect();
    children.sort();
    children
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sql_files(dir: &Path) -> Vec<PathBuf> {
    visible_children(dir, false)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sql")))
        .collect()
}

/// 扫描 Settings 目录下所有 .sql 文件。目录不存在就是本机没有 Navicat(或没存过查询)。
pub fn scan_queries(settings: &Path) -> CoreResult<Vec<NavicatQuery>> {
    if !settings.is_dir() {
        return Err(CoreError::Storage(format!(
            "没有找到本机 Navicat 保存的查询：{}",
            settings.display()
        )));
    }
    let mut queries = Vec::new();
    let type_dirs = visible_children(settings, true)
        .into_iter()
        .flat_map(|project| visible_children(&project, true))
        .flat_map(|sub| visible_children(&sub, true));
    for type_dir in type_dirs {
        for connection in visible_children(&type_dir, true) {
            for database in visible_children(&connection, true) {
                let at = |schema: Option<&Path>, file: PathBuf| NavicatQuery {
                    id: file
                        .strip_prefix(settings)
                        .unwrap_or(&file)
                        .to_string_lossy()
                        .into_owned(),
                    conn_type: name_of(&type_dir),
                    connection: name_of(&connection),
                    database: name_of(&database),
                    schema: schema.map(name_of),
                    name: file
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    path: file,
                };
                queries.extend(sql_files(&database).into_iter().map(|f| at(None, f)));
                for schema in visible_children(&database, true) {
                    queries.extend(sql_files(&schema).into_iter().map(|f| at(Some(&schema), f)));
                }
            }
        }
    }
    Ok(queries)
}

/// 读查询文件:要是 UTF-8 文本,去掉开头的 BOM。
pub fn read_query(query: &NavicatQuery) -> Result<String, String> {
    let bytes = read_source(&query.path).map_err(|e| format!("读取失败：{e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "不是 UTF-8 文本".to_owned())?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryTarget {
    pub connection_id: String,
    pub connection_name: String,
    pub database: String,
    pub schema: Option<String>,
    /// 存到 Basalt 时用的名字(Navicat 的文件名里有 Basalt 不认的字符时会被换掉)。
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchedBy {
    Name,
    Address,
}

fn same_family(navicat: DriverKind, basalt: DriverKind) -> bool {
    let mysql = |d| matches!(d, DriverKind::Mysql | DriverKind::Mariadb);
    navicat == basalt || (mysql(navicat) && mysql(basalt))
}

/// localhost、127.0.0.1、::1 都是本机。
fn host_key(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    match host.as_str() {
        "127.0.0.1" | "::1" | "[::1]" => "localhost".to_owned(),
        _ => host,
    }
}

fn address(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Network { host, port, .. } => format!("{}:{port}", host.trim()),
        Endpoint::File { path } => path.clone(),
    }
}

fn same_server(a: &Endpoint, b: &Endpoint) -> bool {
    match (a, b) {
        (
            Endpoint::Network {
                host: ha, port: pa, ..
            },
            Endpoint::Network {
                host: hb, port: pb, ..
            },
        ) => host_key(ha) == host_key(hb) && pa == pb,
        (Endpoint::File { path: a }, Endpoint::File { path: b }) => a.trim() == b.trim(),
        _ => false,
    }
}

fn username(endpoint: &Endpoint) -> Option<&str> {
    match endpoint {
        Endpoint::Network { username, .. } => Some(username.trim()),
        Endpoint::File { .. } => None,
    }
}

fn names(connections: &[&ConnectionConfig]) -> String {
    connections
        .iter()
        .map(|c| format!("「{}」", c.name))
        .collect::<Vec<_>>()
        .join("、")
}

/// 按 conn.plist 里同名连接的地址找;地址相同的不止一个时,再按用户名缩小。
fn match_by_address<'a>(
    query: &NavicatQuery,
    driver: DriverKind,
    connections: &[&'a ConnectionConfig],
    navicat: &[SourceConnection],
) -> Result<&'a ConnectionConfig, String> {
    let label = display_type(&query.conn_type);
    let missing = format!("Basalt 里没有叫「{}」的 {label} 连接", query.connection);
    let endpoint = navicat
        .iter()
        .filter(|s| s.name == query.connection && s.source_type == label)
        .find_map(|s| match &s.target {
            SourceTarget::Supported { endpoint, .. } => Some(endpoint),
            SourceTarget::Unsupported(_) => None,
        })
        .ok_or_else(|| format!("{missing}，Navicat 的连接配置里也找不到它的地址"))?;
    let same: Vec<&ConnectionConfig> = connections
        .iter()
        .copied()
        .filter(|c| same_family(driver, c.driver) && same_server(&c.endpoint, endpoint))
        .collect();
    let narrowed: Vec<&ConnectionConfig> = if same.len() > 1 {
        same.iter()
            .copied()
            .filter(|c| username(&c.endpoint) == username(endpoint))
            .collect()
    } else {
        same.clone()
    };
    match narrowed.as_slice() {
        [only] => Ok(only),
        [] if same.is_empty() => Err(format!(
            "{missing}，也没有地址是 {} 的连接",
            address(endpoint)
        )),
        _ => Err(format!(
            "地址是 {} 的连接不止一个（{}），不知道该放到哪个下面",
            address(endpoint),
            names(&same)
        )),
    }
}

/// 给一条 Navicat 查询找 Basalt 里的落脚处。
pub fn match_target(
    query: &NavicatQuery,
    connections: &[ConnectionConfig],
    navicat: &[SourceConnection],
) -> Result<(QueryTarget, MatchedBy), String> {
    let driver =
        map_type(&query.conn_type).map_err(|label| format!("Basalt 暂不支持 {label} 的查询"))?;
    let candidates: Vec<&ConnectionConfig> = connections
        .iter()
        .filter(|c| same_family(driver, c.driver))
        .collect();
    let by_name: Vec<&ConnectionConfig> = candidates
        .iter()
        .copied()
        .filter(|c| c.name.trim() == query.connection.trim())
        .collect();
    let (connection, matched_by) = match by_name.as_slice() {
        [only] => (*only, MatchedBy::Name),
        [] => (
            match_by_address(query, driver, &candidates, navicat)?,
            MatchedBy::Address,
        ),
        _ => {
            return Err(format!(
                "Basalt 里叫「{}」的连接不止一个，不知道该放到哪个下面",
                query.connection
            ));
        }
    };
    // 没有 schema 的数据库(MySQL、SQLite)多出来的那层目录认不出,不猜。
    if query.schema.is_some() && driver != DriverKind::Postgres {
        return Err("目录层级和 Navicat 的常见结构对不上，认不出库名".into());
    }
    // PostgreSQL 的查询在 Basalt 里挂在 schema 下;Navicat 里存在库这一层的,
    // 当时用的是默认 search_path,放到 public 下最接近。
    let schema = match (&query.schema, driver) {
        (None, DriverKind::Postgres) => Some("public".to_owned()),
        (schema, _) => schema.clone(),
    };
    Ok((
        QueryTarget {
            connection_id: connection.id.clone(),
            connection_name: connection.name.clone(),
            database: query.database.clone(),
            schema,
            name: sanitize_name(&query.name),
        },
        matched_by,
    ))
}

impl QueryTarget {
    pub fn saved_ref(&self) -> SavedQueryRef {
        SavedQueryRef {
            connection_id: self.connection_id.clone(),
            database: self.database.clone(),
            schema: self.schema.clone(),
            name: self.name.clone(),
        }
    }
}

mod plan;
pub use plan::{
    ImportedQuery, PlannedQuery, QueryImportCandidate, QueryImportReport, QueryImportStatus,
    SkippedQuery, import_planned, plan_queries,
};

#[cfg(test)]
#[path = "queries_tests.rs"]
mod tests;
