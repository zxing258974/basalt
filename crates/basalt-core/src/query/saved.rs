//! 保存的查询:每条一个 .sql 文件,按「连接 id / 库 /(PostgreSQL 的 schema)/ 名字.sql」放。
//! 目录名里的 `/`、`:`、`%` 等字符按百分号编码,库名、schema 名怎么起都能落成文件夹。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local, SecondsFormat};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

const EXTENSION: &str = "sql";
const MAX_NAME_CHARS: usize = 120;
const MAX_NAME_BYTES: usize = 240;
/// 一条查询再大也就是几 MB 的脚本;更大的多半选错了文件,不整个读进内存。
pub const MAX_QUERY_BYTES: u64 = 16 * 1024 * 1024;

/// 一条保存的查询在哪里。PostgreSQL 的查询挂在 schema 下,其余库没有 schema。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryRef {
    pub connection_id: String,
    pub database: String,
    #[serde(default)]
    pub schema: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryInfo {
    pub name: String,
    /// 文件的修改时间,RFC 3339,本机时区。
    pub modified_at: Option<String>,
}

/// 名字要能直接当文件名:去掉首尾空白后不能为空、不能以点开头,不能有路径分隔符和控制字符。
pub fn validate_name(name: &str) -> CoreResult<String> {
    let name = name.trim();
    let invalid = |message: &str| Err(CoreError::InvalidName(message.to_owned()));
    if name.is_empty() {
        return invalid("请给查询起个名字");
    }
    // APFS 的文件名最多 255 字节,加上「.sql」;一个汉字占 3 字节。
    if name.chars().count() > MAX_NAME_CHARS || name.len() > MAX_NAME_BYTES {
        return invalid(&format!(
            "名字太长了，最多 {MAX_NAME_CHARS} 个字（中文最多约 {} 个）",
            MAX_NAME_BYTES / 3
        ));
    }
    if name.starts_with('.') {
        return invalid("名字不能以「.」开头");
    }
    if name.chars().any(is_forbidden) {
        return invalid("名字里不能有 / \\ : * ? \" < > | 或换行这类控制字符");
    }
    if is_reserved(name) {
        return invalid(&format!("「{name}」是 Windows 的保留名，换一个吧"));
    }
    Ok(name.to_owned())
}

/// Windows 文件名里不许出现的字符。macOS 上也一样拦下:同一个名字在两个平台上都存得下。
fn is_forbidden(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
}

/// Windows 的保留设备名(CON、NUL、COM1……),带不带扩展名都不能当文件名。
fn is_reserved(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
    };
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
}

/// 把不能当名字用的字符换成下划线,用于导入别处的查询。
pub fn sanitize_name(name: &str) -> String {
    let replaced: String = name
        .trim()
        .chars()
        .map(|c| if is_forbidden(c) { '_' } else { c })
        .collect();
    let trimmed = replaced.trim_start_matches('.');
    let mut name = String::new();
    for c in trimmed.chars().take(MAX_NAME_CHARS) {
        if name.len() + c.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        name.push(c);
    }
    if name.trim().is_empty() {
        "未命名查询".to_owned()
    } else if is_reserved(&name) {
        format!("_{name}")
    } else {
        name
    }
}

/// 目录名的百分号编码:可以放心落成一层文件夹,不会变成上级目录或隐藏文件。
fn encode_segment(segment: &str) -> String {
    encode_segment_for(segment, cfg!(windows))
}

/// Windows 上还要编码它不收的字符、结尾的点和空格(会被悄悄去掉)、保留设备名的首字母。
/// 只在 Windows 上多编码:macOS 上已有的目录名不变,存过的查询照样找得到。
fn encode_segment_for(segment: &str, windows: bool) -> String {
    let count = segment.chars().count();
    let reserved = windows && is_reserved(segment);
    let mut out = String::with_capacity(segment.len());
    for (index, c) in segment.chars().enumerate() {
        let unsafe_char = matches!(c, '%' | '/' | '\\' | ':') || c.is_control();
        let windows_char = windows
            && (matches!(c, '*' | '?' | '"' | '<' | '>' | '|')
                || (index + 1 == count && matches!(c, '.' | ' '))
                || (index == 0 && reserved));
        if unsafe_char || windows_char || (index == 0 && c == '.') {
            let mut buf = [0u8; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    if out.is_empty() { "%".to_owned() } else { out }
}

fn modified_at(path: &Path) -> Option<String> {
    let modified: SystemTime = std::fs::metadata(path).ok()?.modified().ok()?;
    let local: DateTime<Local> = modified.into();
    Some(local.to_rfc3339_opts(SecondsFormat::Secs, false))
}

fn storage_error(path: &Path, error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("{}：{error}", path.display()))
}

pub struct SavedQueryStore {
    root: PathBuf,
}

impl SavedQueryStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn folder(&self, connection_id: &str, database: &str, schema: Option<&str>) -> PathBuf {
        let folder = self
            .root
            .join(encode_segment(connection_id))
            .join(encode_segment(database));
        match schema {
            Some(schema) => folder.join(encode_segment(schema)),
            None => folder,
        }
    }

    fn file(&self, query: &SavedQueryRef) -> CoreResult<PathBuf> {
        let name = validate_name(&query.name)?;
        Ok(self
            .folder(
                &query.connection_id,
                &query.database,
                query.schema.as_deref(),
            )
            .join(format!("{name}.{EXTENSION}")))
    }

    /// 某个库(PG 是某个 schema)下的查询,按名字排序。文件夹不存在就是没有。
    pub fn list(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<&str>,
    ) -> CoreResult<Vec<SavedQueryInfo>> {
        let folder = self.folder(connection_id, database, schema);
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(storage_error(&folder, e)),
        };
        let mut queries: Vec<SavedQueryInfo> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == EXTENSION))
            .filter_map(|path| {
                let name = path.file_stem()?.to_str()?.to_owned();
                (!name.starts_with('.')).then(|| SavedQueryInfo {
                    modified_at: modified_at(&path),
                    name,
                })
            })
            .collect();
        queries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(queries)
    }

    pub fn exists(&self, query: &SavedQueryRef) -> bool {
        self.file(query).is_ok_and(|path| path.is_file())
    }

    pub fn read(&self, query: &SavedQueryRef) -> CoreResult<String> {
        let path = self.file(query)?;
        let size = match std::fs::metadata(&path) {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CoreError::NotFound(format!(
                    "找不到查询「{}」，可能已被删除或改名",
                    query.name
                )));
            }
            Err(e) => return Err(storage_error(&path, e)),
        };
        if size > MAX_QUERY_BYTES {
            return Err(CoreError::Unsupported(format!(
                "查询「{}」有 {} MB，太大了，请用文本编辑器打开",
                query.name,
                size / 1024 / 1024
            )));
        }
        let bytes = std::fs::read(&path).map_err(|e| storage_error(&path, e))?;
        String::from_utf8(bytes)
            .map_err(|_| CoreError::Unsupported(format!("查询「{}」不是 UTF-8 文本", query.name)))
    }

    /// 写入一条查询。同名的已经有了:`overwrite` 时覆盖,否则报 [`CoreError::AlreadyExists`]。
    /// 先写临时文件再改名,写到一半崩溃也不会留下半个文件。
    pub fn save(
        &self,
        query: &SavedQueryRef,
        sql: &str,
        overwrite: bool,
    ) -> CoreResult<SavedQueryInfo> {
        let path = self.file(query)?;
        if !overwrite && path.exists() {
            return Err(CoreError::AlreadyExists(format!(
                "这个库下已经有叫「{}」的查询了",
                query.name.trim()
            )));
        }
        let folder = path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(folder).map_err(|e| storage_error(folder, e))?;
        let tmp = path.with_extension("sql.tmp");
        std::fs::write(&tmp, sql).map_err(|e| storage_error(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| storage_error(&path, e))?;
        Ok(SavedQueryInfo {
            name: validate_name(&query.name)?,
            modified_at: modified_at(&path),
        })
    }

    pub fn delete(&self, query: &SavedQueryRef) -> CoreResult<()> {
        let path = self.file(query)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(storage_error(&path, e)),
        }
    }
}

#[cfg(test)]
#[path = "saved_tests.rs"]
mod tests;
