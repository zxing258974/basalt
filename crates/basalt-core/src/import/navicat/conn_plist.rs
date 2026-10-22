//! 本机 Navicat 16 的 conn.plist 与分组文件 vgroup.json。
//!
//! 这两个文件都没有公开的格式说明。conn.plist 实测是 `{"0": {"0": {类型: {连接名: 字段}}}, "Snowflake": …}`,
//! 字段里有 host、port(字符串)、username、defaultdatabase、usetunnel、usessl 等,**没有密码**。

use std::collections::HashMap;
use std::io::Cursor;

use plist::{Dictionary, Value};

use super::{HTTP_TUNNEL_WARNING, RawConnection, into_sources};
use crate::error::{CoreError, CoreResult};
use crate::import::{SourceConnection, SourcePassword};

pub fn parse_conn_plist(bytes: &[u8]) -> CoreResult<Vec<SourceConnection>> {
    let root = Value::from_reader(Cursor::new(bytes)).map_err(|e| {
        CoreError::Unsupported(format!("不是有效的 Navicat 连接配置（conn.plist）：{e}"))
    })?;
    let root = root.as_dictionary().ok_or_else(|| {
        CoreError::Unsupported("不是有效的 Navicat 连接配置（conn.plist）：根节点不是字典".into())
    })?;
    // 根 → 项目 → 类型表:每层都可能有多项,逐层展开,只认「连接名 → 字段字典」的类型表。
    let raws = root
        .values()
        .filter_map(Value::as_dictionary)
        .flat_map(Dictionary::values)
        .filter_map(Value::as_dictionary)
        .flat_map(Dictionary::iter)
        .filter_map(|(conn_type, table)| Some((conn_type, connection_table(table)?)))
        .flat_map(|(conn_type, table)| {
            table
                .into_iter()
                .map(move |(name, fields)| raw_connection(conn_type, name, fields))
        })
        .collect();
    Ok(into_sources(raws))
}

/// 每一项都是字段字典才算类型表;Snowflake 那种别的结构直接跳过。
fn connection_table(value: &Value) -> Option<Vec<(&String, &Dictionary)>> {
    value
        .as_dictionary()?
        .iter()
        .map(|(name, fields)| Some((name, fields.as_dictionary()?)))
        .collect()
}

fn raw_connection(conn_type: &str, name: &str, fields: &Dictionary) -> RawConnection {
    let text = |key: &str| match fields.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Integer(n)) => n.to_string(),
        _ => String::new(),
    };
    let flag = |key: &str| fields.get(key).and_then(Value::as_boolean).unwrap_or(false);
    // conn.plist 里隧道和 SSL 的具体字段没见过实例,不猜字段名,只提醒。
    let notes = [
        (flag("usetunnel"), PLIST_SSH_WARNING),
        (flag("usessl"), PLIST_SSL_WARNING),
        (flag("usehttptunnel"), HTTP_TUNNEL_WARNING),
    ]
    .into_iter()
    .filter(|(used, _)| *used)
    .map(|(_, note)| note.to_owned())
    .collect();
    RawConnection {
        name: name.to_owned(),
        conn_type: conn_type.to_owned(),
        host: text("host"),
        port: text("port"),
        username: text("username"),
        database: text("defaultdatabase"),
        // 用户这份里 SQLite 表是空的,字段名没见过;按 .ncx 的 DatabaseFileName 推测几个候选。
        file_path: ["databasefilename", "databasefile", "filename"]
            .into_iter()
            .map(text)
            .find(|path| !path.trim().is_empty())
            .unwrap_or_default(),
        password: SourcePassword::Absent,
        notes,
        ..RawConnection::default()
    }
}

pub const PLIST_SSH_WARNING: &str = "SSH 隧道没有迁移（本机 Navicat 配置里隧道的写法没有核实过）：请在 Navicat 里导出 .ncx 再导入，或者导入后在「SSH」页补上";
pub const PLIST_SSL_WARNING: &str = "SSL 设置没有迁移（本机 Navicat 配置里 SSL 的写法没有核实过）：请在 Navicat 里导出 .ncx 再导入，或者导入后在「SSL」页补上";

/// 分组文件的格式没见过实例(用户这份是空的 `{}`),按常见的「组名 + 子项数组」结构尽力识别:
/// 组节点是带 name 和子项数组的对象,子项可以是连接名字符串、带名字的对象或者嵌套的组。
/// 认不出就返回空表,连接照常导入,只是不带分组。
pub fn parse_vgroups(json: &str) -> HashMap<String, String> {
    let mut groups = HashMap::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
        collect_groups(&value, None, &mut groups);
    }
    groups
}

const NAME_KEYS: [&str; 4] = ["name", "Name", "groupName", "GroupName"];
const CHILD_KEYS: [&str; 6] = [
    "children",
    "Children",
    "items",
    "Items",
    "connections",
    "Connections",
];
const MEMBER_KEYS: [&str; 6] = [
    "connectionName",
    "ConnectionName",
    "connName",
    "name",
    "Name",
    "connection",
];

fn group_parts(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<(&str, &Vec<serde_json::Value>)> {
    let name = NAME_KEYS.iter().find_map(|k| object.get(*k)?.as_str())?;
    let children = CHILD_KEYS.iter().find_map(|k| object.get(*k)?.as_array())?;
    Some((name, children))
}

fn collect_groups(
    value: &serde_json::Value,
    group: Option<&str>,
    out: &mut HashMap<String, String>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_groups(item, group, out);
            }
        }
        serde_json::Value::Object(object) => {
            if let Some((name, children)) = group_parts(object) {
                for child in children {
                    collect_groups(child, Some(name), out);
                }
            } else if let Some(group) = group {
                if let Some(member) = MEMBER_KEYS.iter().find_map(|k| object.get(*k)?.as_str()) {
                    out.insert(member.to_owned(), group.to_owned());
                }
            } else {
                for child in object.values() {
                    collect_groups(child, None, out);
                }
            }
        }
        serde_json::Value::String(member) => {
            if let Some(group) = group {
                out.insert(member.clone(), group.to_owned());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "conn_plist_tests.rs"]
mod tests;
