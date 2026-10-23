//! 键浏览:按模式和类型扫描键,读一个键的值。
//!
//! 只用 SCAN / HSCAN / SSCAN 这类分批的命令,不用 KEYS:大库上 KEYS 会卡住整个服务器。
//! 集合类的值一次最多读 [`MAX_ITEMS`] 项,字符串最多读 [`MAX_STRING_BYTES`] 字节,够看也不拖垮界面。

use std::collections::BTreeMap;

use redis::{Value, cmd, pipe};
use serde::{Deserialize, Serialize};

use super::RedisSession;
use crate::error::{CoreError, CoreResult};
use crate::table::{ColumnInfo, TablePage, ValueType};
use crate::value::CellValue;

/// 集合类(hash、list、set、zset、stream)一次最多读这么多项。
pub const MAX_ITEMS: usize = 1000;
/// 字符串最多读这么多字节。
pub const MAX_STRING_BYTES: usize = 1024 * 1024;
/// 一次扫描最多走这么多轮 SCAN:模式很偏时也及时回来,让界面显示「继续扫描」。
const MAX_SCAN_ROUNDS: usize = 50;
/// 每轮 SCAN 建议服务器看多少个键。
const SCAN_COUNT: usize = 1000;

/// 键的名字。Redis 的键是二进制的:是 UTF-8 时 `id` 是 `s:原文`,不是时是 `x:十六进制`。
/// 前端拿 `id` 回传,显示用 `name`(不是 UTF-8 的按十六进制显示)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyName {
    pub id: String,
    pub name: String,
    pub binary: bool,
}

impl KeyName {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self {
                id: format!("s:{text}"),
                name: text.to_owned(),
                binary: false,
            },
            Err(_) => {
                let hex = hex::encode(bytes);
                Self {
                    id: format!("x:{hex}"),
                    name: format!("0x{hex}"),
                    binary: true,
                }
            }
        }
    }
}

/// 前端回传的键 id → 键的原始字节。
pub fn key_bytes(id: &str) -> CoreResult<Vec<u8>> {
    if let Some(text) = id.strip_prefix("s:") {
        return Ok(text.as_bytes().to_vec());
    }
    if let Some(hex) = id.strip_prefix("x:") {
        return hex::decode(hex)
            .map_err(|_| CoreError::InvalidEdit(format!("键名「{id}」不是有效的十六进制")));
    }
    Err(CoreError::InvalidEdit(format!("认不出键「{id}」")))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeySummary {
    pub key: KeyName,
    /// TYPE 的回复:string、hash、list、set、zset、stream,或者模块的类型名。
    pub kind: String,
    /// 剩余的毫秒数;不过期为 `None`。
    pub ttl_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    /// SCAN 的 MATCH 模式,前端已经把普通文字包成 `*文字*`。
    pub pattern: String,
    /// 只要某一种类型。
    pub kind: Option<String>,
    /// 上次扫描停下的游标,从头扫是 "0"。
    pub cursor: String,
    /// 凑够这么多个就先回来。
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanPage {
    pub keys: Vec<KeySummary>,
    /// 下次接着扫的游标;"0" 表示扫完了。
    pub cursor: String,
}

/// 一个键的值。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KeyValue {
    /// 字符串,是 UTF-8 文本。
    #[serde(rename_all = "camelCase")]
    Text { text: String, truncated: bool },
    /// 字符串,不是 UTF-8:十六进制,不带 0x。
    #[serde(rename_all = "camelCase")]
    Bytes { hex: String, truncated: bool },
    /// hash、list、set、zset、stream 按表格显示。
    #[serde(rename_all = "camelCase")]
    Table { page: TablePage, truncated: bool },
    /// 模块的类型(RedisJSON、时间序列……)。
    #[serde(rename_all = "camelCase")]
    Unsupported { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyDetail {
    pub key: KeyName,
    pub kind: String,
    pub ttl_ms: Option<i64>,
    /// 字符串的字节数,集合类的项数。
    pub length: Option<u64>,
    /// MEMORY USAGE;没有权限或者不支持时为 `None`。
    pub memory: Option<u64>,
    /// OBJECT ENCODING。
    pub encoding: Option<String>,
    pub value: KeyValue,
}

/// PTTL 的回复:-1 不过期,-2 键不存在。
fn ttl(pttl: i64) -> Option<i64> {
    (pttl >= 0).then_some(pttl)
}

fn bytes_of(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::BulkString(bytes) => Some(bytes.clone()),
        Value::SimpleString(text) => Some(text.as_bytes().to_vec()),
        Value::VerbatimString { text, .. } => Some(text.as_bytes().to_vec()),
        Value::Int(n) => Some(n.to_string().into_bytes()),
        _ => None,
    }
}

/// 值里的一段字节:是 UTF-8 就当文本,不是就是二进制。
pub fn cell(bytes: Vec<u8>) -> CellValue {
    match String::from_utf8(bytes) {
        Ok(text) => CellValue::Text(text),
        Err(error) => CellValue::Bytes(error.into_bytes()),
    }
}

fn column(name: &str, data_type: &str, key: bool, value_type: ValueType) -> ColumnInfo {
    ColumnInfo {
        is_primary_key: key,
        nullable: false,
        value_type,
        ..ColumnInfo::new(name, data_type)
    }
}

impl RedisSession {
    /// 支持 `SCAN … TYPE` 的版本(Redis 6 起;Valkey 都支持)。
    fn scan_has_type(&self) -> bool {
        let major = self
            .server()
            .version
            .split('.')
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        major >= 6 || self.server().product != "Redis"
    }

    /// 扫描一批键,带上类型和剩余时间。按需求的数量凑够,或者扫完、走够轮数就回来。
    pub async fn scan_keys(&self, db: u32, request: &ScanRequest) -> CoreResult<ScanPage> {
        let pattern = if request.pattern.is_empty() {
            "*"
        } else {
            &request.pattern
        };
        let by_server = self.scan_has_type();
        let mut cursor = request.cursor.clone();
        let mut found: Vec<Vec<u8>> = Vec::new();
        for _ in 0..MAX_SCAN_ROUNDS {
            let mut scan = cmd("SCAN");
            scan.arg(&cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(SCAN_COUNT);
            if let (true, Some(kind)) = (by_server, &request.kind) {
                scan.arg("TYPE").arg(kind);
            }
            let (next, keys): (String, Vec<Vec<u8>>) = self.query(db, &scan).await?;
            found.extend(keys);
            cursor = next;
            if cursor == "0" || found.len() >= request.count.max(1) {
                break;
            }
        }
        let keys = self.describe_keys(db, &found).await?;
        let keys = keys
            .into_iter()
            .filter(|k| request.kind.as_ref().is_none_or(|kind| &k.kind == kind))
            .collect();
        Ok(ScanPage { keys, cursor })
    }

    /// 一批键的类型和剩余时间:一个流水线,一次往返。期间被删掉的键不要。
    async fn describe_keys(&self, db: u32, keys: &[Vec<u8>]) -> CoreResult<Vec<KeySummary>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut batch = pipe();
        for key in keys {
            batch.cmd("TYPE").arg(key).cmd("PTTL").arg(key);
        }
        let replies: Vec<Value> = self.query_pipe(db, &batch).await?;
        Ok(keys
            .iter()
            .zip(replies.chunks(2))
            .filter_map(|(key, pair)| {
                let kind = match &pair[0] {
                    Value::SimpleString(kind) => kind.clone(),
                    other => String::from_utf8(bytes_of(other)?).ok()?,
                };
                let pttl = match pair[1] {
                    Value::Int(n) => n,
                    _ => -1,
                };
                (kind != "none" && pttl != -2).then(|| KeySummary {
                    key: KeyName::from_bytes(key),
                    kind,
                    ttl_ms: ttl(pttl),
                })
            })
            .collect())
    }

    /// 读一个键:类型、剩余时间、长度、占用内存、编码和值。键不存在时报 `NotFound`。
    pub async fn read_key(&self, db: u32, id: &str) -> CoreResult<KeyDetail> {
        let key = key_bytes(id)?;
        let (kind, pttl): (String, i64) = self
            .query_pipe(db, pipe().cmd("TYPE").arg(&key).cmd("PTTL").arg(&key))
            .await?;
        if kind == "none" {
            return Err(CoreError::NotFound(
                "这个键已经不存在了（可能过期了，或者被删了）".into(),
            ));
        }
        // 这两项拿不到也不要紧:云上常禁用 OBJECT、MEMORY。
        let encoding = self
            .query::<Option<String>>(db, cmd("OBJECT").arg("ENCODING").arg(&key))
            .await
            .ok()
            .flatten();
        let memory = self
            .query::<Option<u64>>(db, cmd("MEMORY").arg("USAGE").arg(&key))
            .await
            .ok()
            .flatten();
        let (length, value) = self.read_value(db, &key, &kind).await?;
        Ok(KeyDetail {
            key: KeyName::from_bytes(&key),
            kind,
            ttl_ms: ttl(pttl),
            length,
            memory,
            encoding,
            value,
        })
    }

    async fn read_value(
        &self,
        db: u32,
        key: &[u8],
        kind: &str,
    ) -> CoreResult<(Option<u64>, KeyValue)> {
        let name = String::from_utf8_lossy(key).into_owned();
        Ok(match kind {
            "string" => {
                let length: u64 = self.query(db, cmd("STRLEN").arg(key)).await?;
                let bytes: Vec<u8> = self
                    .query(
                        db,
                        cmd("GETRANGE").arg(key).arg(0).arg(MAX_STRING_BYTES - 1),
                    )
                    .await?;
                let truncated = length as usize > MAX_STRING_BYTES;
                let value = match String::from_utf8(bytes) {
                    Ok(text) => KeyValue::Text { text, truncated },
                    // 截在一个字的中间时也算文本:去掉被截断的半个字。
                    Err(error) if truncated && error.utf8_error().error_len().is_none() => {
                        let valid = error.utf8_error().valid_up_to();
                        let mut bytes = error.into_bytes();
                        bytes.truncate(valid);
                        KeyValue::Text {
                            text: String::from_utf8(bytes).unwrap_or_default(),
                            truncated,
                        }
                    }
                    Err(error) => KeyValue::Bytes {
                        hex: hex::encode(error.into_bytes()),
                        truncated,
                    },
                };
                (Some(length), value)
            }
            "hash" => {
                let length: u64 = self.query(db, cmd("HLEN").arg(key)).await?;
                let pairs = self.scan_collection(db, "HSCAN", key, 2).await?;
                let mut rows: Vec<Vec<CellValue>> = pairs
                    .into_iter()
                    .map(|mut pair| {
                        let value = pair.pop().unwrap_or_default();
                        let field = pair.pop().unwrap_or_default();
                        vec![cell(field), cell(value)]
                    })
                    .collect();
                rows.sort_by(|a, b| sort_key(&a[0]).cmp(&sort_key(&b[0])));
                let columns = vec![
                    column("field", "字段", true, ValueType::Text),
                    column("value", "值", false, ValueType::Text),
                ];
                table(
                    columns,
                    rows,
                    length,
                    format!("HSCAN {name} 0 COUNT {SCAN_COUNT}"),
                    None,
                )
            }
            "list" => {
                let length: u64 = self.query(db, cmd("LLEN").arg(key)).await?;
                let items: Vec<Vec<u8>> = self
                    .query(db, cmd("LRANGE").arg(key).arg(0).arg(MAX_ITEMS - 1))
                    .await?;
                let rows = items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| vec![CellValue::Int(index as i64), cell(item)])
                    .collect();
                let columns = vec![
                    ColumnInfo {
                        is_generated: true,
                        has_default: true,
                        ..column("index", "下标", true, ValueType::Integer { unsigned: true })
                    },
                    column("value", "值", false, ValueType::Text),
                ];
                table(
                    columns,
                    rows,
                    length,
                    format!("LRANGE {name} 0 {}", MAX_ITEMS - 1),
                    None,
                )
            }
            "set" => {
                let length: u64 = self.query(db, cmd("SCARD").arg(key)).await?;
                let members = self.scan_collection(db, "SSCAN", key, 1).await?;
                let mut rows: Vec<Vec<CellValue>> = members
                    .into_iter()
                    .map(|mut one| vec![cell(one.pop().unwrap_or_default())])
                    .collect();
                rows.sort_by(|a, b| sort_key(&a[0]).cmp(&sort_key(&b[0])));
                let columns = vec![column("member", "成员", true, ValueType::Text)];
                table(
                    columns,
                    rows,
                    length,
                    format!("SSCAN {name} 0 COUNT {SCAN_COUNT}"),
                    None,
                )
            }
            "zset" => {
                let length: u64 = self.query(db, cmd("ZCARD").arg(key)).await?;
                let flat: Vec<Vec<u8>> = self
                    .query(
                        db,
                        cmd("ZRANGE")
                            .arg(key)
                            .arg(0)
                            .arg(MAX_ITEMS - 1)
                            .arg("WITHSCORES"),
                    )
                    .await?;
                let rows = flat
                    .chunks(2)
                    .map(|pair| {
                        let score = String::from_utf8_lossy(&pair[1]).into_owned();
                        vec![cell(pair[0].clone()), CellValue::Decimal(score)]
                    })
                    .collect();
                let columns = vec![
                    column("member", "成员", true, ValueType::Text),
                    column("score", "分数", false, ValueType::Float),
                ];
                table(
                    columns,
                    rows,
                    length,
                    format!("ZRANGE {name} 0 {} WITHSCORES", MAX_ITEMS - 1),
                    None,
                )
            }
            "stream" => {
                let length: u64 = self.query(db, cmd("XLEN").arg(key)).await?;
                let entries: Vec<(String, Vec<Vec<u8>>)> = self
                    .query(
                        db,
                        cmd("XRANGE")
                            .arg(key)
                            .arg("-")
                            .arg("+")
                            .arg("COUNT")
                            .arg(MAX_ITEMS),
                    )
                    .await?;
                let rows = entries
                    .into_iter()
                    .map(|(id, fields)| {
                        vec![CellValue::Text(id), CellValue::Json(fields_json(&fields))]
                    })
                    .collect();
                let columns = vec![
                    column("id", "消息 ID", true, ValueType::Text),
                    column("fields", "字段", false, ValueType::Text),
                ];
                table(
                    columns,
                    rows,
                    length,
                    format!("XRANGE {name} - + COUNT {MAX_ITEMS}"),
                    Some("stream 的消息只能追加和删除，不能改；在命令行里用 XADD、XDEL".into()),
                )
            }
            other => (
                None,
                KeyValue::Unsupported {
                    reason: format!(
                        "Basalt 还显示不了 {other} 类型的值，可以在命令行里用这个类型自己的命令查看"
                    ),
                },
            ),
        })
    }

    /// HSCAN / SSCAN 直到读够 [`MAX_ITEMS`] 项或扫完;`width` 是一项占几个回复(hash 是 2)。
    async fn scan_collection(
        &self,
        db: u32,
        command: &str,
        key: &[u8],
        width: usize,
    ) -> CoreResult<Vec<Vec<Vec<u8>>>> {
        let mut cursor = "0".to_owned();
        let mut items = Vec::new();
        loop {
            let (next, flat): (String, Vec<Vec<u8>>) = self
                .query(
                    db,
                    cmd(command)
                        .arg(key)
                        .arg(&cursor)
                        .arg("COUNT")
                        .arg(SCAN_COUNT),
                )
                .await?;
            items.extend(flat.chunks(width).map(<[Vec<u8>]>::to_vec));
            cursor = next;
            if cursor == "0" || items.len() >= MAX_ITEMS {
                break;
            }
        }
        // HSCAN、SSCAN 可能把同一项回两次,按内容去重。
        let mut seen = BTreeMap::new();
        items.retain(|item| seen.insert(item[0].clone(), ()).is_none());
        items.truncate(MAX_ITEMS);
        Ok(items)
    }
}

fn table(
    columns: Vec<ColumnInfo>,
    rows: Vec<Vec<CellValue>>,
    length: u64,
    command: String,
    read_only: Option<String>,
) -> (Option<u64>, KeyValue) {
    let truncated = (rows.len() as u64) < length;
    let page = TablePage {
        columns,
        rows,
        has_more: truncated,
        read_only_reason: read_only,
        sql: command,
    };
    (Some(length), KeyValue::Table { page, truncated })
}

fn sort_key(cell: &CellValue) -> Vec<u8> {
    match cell {
        CellValue::Text(text) => text.as_bytes().to_vec(),
        CellValue::Bytes(bytes) => bytes.clone(),
        _ => Vec::new(),
    }
}

/// stream 一条消息的字段按原来的顺序写成 JSON 对象;不是 UTF-8 的值写成 0x 十六进制。
fn fields_json(fields: &[Vec<u8>]) -> String {
    let text = |bytes: &[u8]| match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => format!("0x{}", hex::encode(bytes)),
    };
    let pairs: Vec<String> = fields
        .chunks(2)
        .map(|pair| {
            let field = serde_json::to_string(&text(&pair[0])).unwrap_or_default();
            let value = serde_json::to_string(&text(pair.get(1).map_or(&[][..], Vec::as_slice)))
                .unwrap_or_default();
            format!("{field}:{value}")
        })
        .collect();
    format!("{{{}}}", pairs.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip_through_their_id() {
        let text = KeyName::from_bytes("用户:1001".as_bytes());
        assert_eq!(text.id, "s:用户:1001");
        assert!(!text.binary);
        assert_eq!(key_bytes(&text.id).unwrap(), "用户:1001".as_bytes());

        let binary = KeyName::from_bytes(&[0xff, 0x00, 0x41]);
        assert_eq!(binary.id, "x:ff0041");
        assert_eq!(binary.name, "0xff0041");
        assert!(binary.binary);
        assert_eq!(key_bytes(&binary.id).unwrap(), vec![0xff, 0x00, 0x41]);

        assert!(key_bytes("x:zz").is_err());
        assert!(key_bytes("user:1").is_err());
    }

    #[test]
    fn stream_fields_keep_their_order() {
        let fields = vec![
            b"status".to_vec(),
            b"paid".to_vec(),
            b"amount".to_vec(),
            b"9.5".to_vec(),
        ];
        assert_eq!(fields_json(&fields), r#"{"status":"paid","amount":"9.5"}"#);
        assert_eq!(
            fields_json(&[b"raw".to_vec(), vec![0xff]]),
            r#"{"raw":"0xff"}"#
        );
    }

    #[test]
    fn values_are_text_when_they_are_utf8() {
        assert_eq!(
            cell("黄勇".as_bytes().to_vec()),
            CellValue::Text("黄勇".into())
        );
        assert_eq!(cell(vec![0xc3]), CellValue::Bytes(vec![0xc3]));
    }
}
