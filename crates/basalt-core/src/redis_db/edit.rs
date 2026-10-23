//! 改 Redis 的值:表格里的修改(hash、list、set、zset)、字符串、改名、过期时间、删除、新建。
//!
//! 表格里的一批修改换成 [`edit.lua`](edit.lua) 的一串步骤:先核对每一项还是读出来时的样子,
//! 全对得上才一起写入,对不上一项都不写——和 SQL 那边「每条语句恰好命中一行,否则整批回滚」一个意思。
//! 预览给人看的是等效的命令(核对那几步不列),提交时重新生成一遍,和预览不一样就不执行。

use std::collections::BTreeSet;

use redis::{Script, cmd};
use serde::Deserialize;

use super::RedisSession;
use super::keys::{KeyName, key_bytes};
use crate::error::{CoreError, CoreResult};
use crate::table::{ApplyOutcome, EditValues, RowChange, RowKey};
use crate::value::CellValue;

const SCRIPT: &str = include_str!("edit.lua");
/// list 删除时先把要删的元素换成它,再一次 LREM 掉(下标在删除过程中不会错位)。和 edit.lua 一致。
const TOMBSTONE: &[u8] = b"\0basalt:deleted";

/// 脚本的一步。
struct Step {
    op: &'static str,
    a: Vec<u8>,
    b: Vec<u8>,
    /// 核对不过时说的是哪一项,如「字段 name」。
    what: String,
}

impl Step {
    fn new(op: &'static str, a: impl Into<Vec<u8>>, b: impl Into<Vec<u8>>, what: String) -> Self {
        Self {
            op,
            a: a.into(),
            b: b.into(),
            what,
        }
    }

    fn write(op: &'static str, a: impl Into<Vec<u8>>, b: impl Into<Vec<u8>>) -> Self {
        Self::new(op, a, b, String::new())
    }
}

/// 一批修改:脚本的步骤和给人看的命令。
struct Plan {
    steps: Vec<Step>,
    commands: Vec<String>,
}

impl Plan {
    fn new(kind: &str) -> Self {
        Self {
            steps: vec![Step::new("type", kind, "", String::new())],
            commands: Vec::new(),
        }
    }

    fn check(
        &mut self,
        op: &'static str,
        a: impl Into<Vec<u8>>,
        b: impl Into<Vec<u8>>,
        what: String,
    ) {
        self.steps.push(Step::new(op, a, b, what));
    }

    fn write(&mut self, key: &[u8], command: &str, op: &'static str, args: &[&[u8]]) {
        self.commands.push(command_text(command, key, args));
        let a = args.first().map(|a| a.to_vec()).unwrap_or_default();
        let b = args.get(1).map(|b| b.to_vec()).unwrap_or_default();
        self.steps.push(Step::write(op, a, b));
    }
}

/// 显示出来看不见、或者会把后面的字倒过来显示的字:控制字符、BOM、双向文字的控制符。
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{feff}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// 照 redis-cli 的写法:普通文字原样,有空白、引号或不是 UTF-8 的加双引号并转义。
pub fn quote(bytes: &[u8]) -> String {
    let plain = !bytes.is_empty()
        && std::str::from_utf8(bytes).is_ok_and(|text| {
            !text
                .chars()
                .any(|c| c.is_whitespace() || hidden(c) || matches!(c, '"' | '\'' | '\\'))
        });
    if plain {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut out = String::from('"');
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                // 看不见的字照它的 UTF-8 字节写,和 redis-cli 一样能原样贴回去。
                c if hidden(c) => {
                    for byte in c.encode_utf8(&mut [0; 4]).bytes() {
                        out.push_str(&format!("\\x{byte:02x}"));
                    }
                }
                c => out.push(c),
            }
        }
        for byte in chunk.invalid() {
            out.push_str(&format!("\\x{byte:02x}"));
        }
    }
    out.push('"');
    out
}

fn command_text(command: &str, key: &[u8], args: &[&[u8]]) -> String {
    let mut parts = vec![command.to_owned(), quote(key)];
    parts.extend(args.iter().map(|a| quote(a)));
    parts.join(" ")
}

/// 读出来的原值 → 字节。
fn original(key: &RowKey, column: &str) -> CoreResult<Vec<u8>> {
    match key.get(column) {
        Some(CellValue::Text(text) | CellValue::Decimal(text)) => Ok(text.as_bytes().to_vec()),
        Some(CellValue::Bytes(bytes)) => Ok(bytes.clone()),
        Some(CellValue::Int(n)) => Ok(n.to_string().into_bytes()),
        _ => Err(CoreError::InvalidEdit(format!(
            "缺少「{column}」的原值，没法核对"
        ))),
    }
}

/// 新值;Redis 没有 NULL。没改的列返回 `None`。
fn edited(values: &EditValues, column: &str) -> CoreResult<Option<Vec<u8>>> {
    match values.get(column) {
        None => Ok(None),
        Some(None) => Err(CoreError::InvalidEdit(
            "Redis 里没有 NULL，可以填空字符串".into(),
        )),
        Some(Some(text)) => Ok(Some(text.as_bytes().to_vec())),
    }
}

fn required(values: &EditValues, column: &str, label: &str) -> CoreResult<Vec<u8>> {
    edited(values, column)?
        .ok_or_else(|| CoreError::InvalidEdit(format!("新增的行要填「{label}」")))
}

/// zset 的分数:数字,或 inf / -inf。
/// 分数要是 Redis 认的数:写入阶段出错时脚本不会回滚,所以 Redis 会拒绝的写法在这里先拦下。
/// 太大变成无穷、太小变成 0 的(`1e400`、`1e-400`)Redis 也不认;无穷要明写 `inf`。
fn score(bytes: Vec<u8>) -> CoreResult<Vec<u8>> {
    let text = String::from_utf8_lossy(&bytes).trim().to_owned();
    let infinite = matches!(text.to_ascii_lowercase().as_str(), "inf" | "+inf" | "-inf");
    let finite = text.parse::<f64>().is_ok_and(|n| {
        let mantissa = text.split(['e', 'E']).next().unwrap_or_default();
        let underflow = n == 0.0 && mantissa.bytes().any(|b| (b'1'..=b'9').contains(&b));
        n.is_finite() && !underflow
    });
    if infinite || finite {
        Ok(text.into_bytes())
    } else {
        Err(CoreError::InvalidEdit(format!(
            "分数「{text}」不是 Redis 能用的数"
        )))
    }
}

/// 过期时间最长 100 年;再长 Redis 会因为算出的时刻溢出而拒绝。
pub const MAX_TTL_SECONDS: u64 = 100 * 365 * 86_400;

fn check_ttl(seconds: u64) -> CoreResult<()> {
    match seconds {
        0 => Err(CoreError::InvalidEdit("过期时间至少 1 秒".into())),
        s if s > MAX_TTL_SECONDS => Err(CoreError::InvalidEdit(
            "过期时间最长 100 年；想一直留着就选不过期".into(),
        )),
        _ => Ok(()),
    }
}

fn shown(bytes: &[u8]) -> String {
    quote(bytes)
}

/// 同一批修改里同一个字段 / 成员不能写两次:核对都在写入之前,后写的会悄悄盖掉先写的。
fn once(written: &mut BTreeSet<Vec<u8>>, item: &[u8], label: &str) -> CoreResult<()> {
    if written.insert(item.to_vec()) {
        Ok(())
    } else {
        Err(CoreError::InvalidEdit(format!(
            "同一批修改里{label} {} 出现了两次",
            shown(item)
        )))
    }
}

fn plan(kind: &str, key: &[u8], changes: &[RowChange]) -> CoreResult<Plan> {
    let mut plan = Plan::new(kind);
    let mut written = BTreeSet::new();
    let mut deleted_from_list = false;
    for change in changes {
        match kind {
            "hash" => hash_change(&mut plan, key, change, &mut written)?,
            "list" => deleted_from_list |= list_change(&mut plan, key, change)?,
            "set" => set_change(&mut plan, key, change, &mut written)?,
            "zset" => zset_change(&mut plan, key, change, &mut written)?,
            "stream" => {
                return Err(CoreError::ReadOnly(
                    "stream 的消息只能追加和删除，不能改；在命令行里用 XADD、XDEL".into(),
                ));
            }
            other => {
                return Err(CoreError::ReadOnly(format!(
                    "Basalt 改不了 {other} 类型的值"
                )));
            }
        }
    }
    if deleted_from_list {
        plan.commands
            .push(command_text("LREM", key, &[b"0", TOMBSTONE]));
    }
    Ok(plan)
}

fn hash_change(
    plan: &mut Plan,
    key: &[u8],
    change: &RowChange,
    written: &mut BTreeSet<Vec<u8>>,
) -> CoreResult<()> {
    match change {
        RowChange::Update { key: row, values } => {
            let (field, value) = (original(row, "field")?, original(row, "value")?);
            let new_field = edited(values, "field")?.unwrap_or_else(|| field.clone());
            let new_value = edited(values, "value")?.unwrap_or_else(|| value.clone());
            plan.check(
                "eq_hget",
                field.clone(),
                value,
                format!("字段 {}", shown(&field)),
            );
            once(written, &new_field, "字段")?;
            if new_field == field {
                plan.write(key, "HSET", "hset", &[&field, &new_value]);
            } else {
                plan.check(
                    "no_hfield",
                    new_field.clone(),
                    "",
                    format!("字段 {}", shown(&new_field)),
                );
                plan.write(key, "HDEL", "hdel", &[&field]);
                plan.write(key, "HSET", "hset", &[&new_field, &new_value]);
            }
        }
        RowChange::Delete { key: row } => {
            let (field, value) = (original(row, "field")?, original(row, "value")?);
            plan.check(
                "eq_hget",
                field.clone(),
                value,
                format!("字段 {}", shown(&field)),
            );
            plan.write(key, "HDEL", "hdel", &[&field]);
        }
        RowChange::Insert { values } => {
            let field = required(values, "field", "字段")?;
            let value = edited(values, "value")?.unwrap_or_default();
            once(written, &field, "字段")?;
            plan.check(
                "no_hfield",
                field.clone(),
                "",
                format!("字段 {}", shown(&field)),
            );
            plan.write(key, "HSET", "hset", &[&field, &value]);
        }
    }
    Ok(())
}

/// 返回这一项是不是删除(删除最后要补一句 LREM)。
fn list_change(plan: &mut Plan, key: &[u8], change: &RowChange) -> CoreResult<bool> {
    match change {
        RowChange::Update { key: row, values } => {
            let (index, value) = (original(row, "index")?, original(row, "value")?);
            let what = format!("下标 {} 的元素", String::from_utf8_lossy(&index));
            plan.check("eq_lindex", index.clone(), value, what);
            if let Some(new_value) = edited(values, "value")? {
                plan.write(key, "LSET", "lset", &[&index, &new_value]);
            }
            Ok(false)
        }
        RowChange::Delete { key: row } => {
            let (index, value) = (original(row, "index")?, original(row, "value")?);
            let what = format!("下标 {} 的元素", String::from_utf8_lossy(&index));
            plan.check("eq_lindex", index.clone(), value, what);
            plan.commands
                .push(command_text("LSET", key, &[&index, TOMBSTONE]));
            plan.steps.push(Step::write("lmark", index, Vec::new()));
            Ok(true)
        }
        RowChange::Insert { values } => {
            let value = edited(values, "value")?.unwrap_or_default();
            plan.write(key, "RPUSH", "rpush", &[&value]);
            Ok(false)
        }
    }
}

fn set_change(
    plan: &mut Plan,
    key: &[u8],
    change: &RowChange,
    written: &mut BTreeSet<Vec<u8>>,
) -> CoreResult<()> {
    match change {
        RowChange::Update { key: row, values } => {
            let member = original(row, "member")?;
            let Some(new_member) = edited(values, "member")? else {
                return Ok(());
            };
            plan.check(
                "is_member",
                member.clone(),
                "",
                format!("成员 {}", shown(&member)),
            );
            plan.check(
                "no_member",
                new_member.clone(),
                "",
                format!("成员 {}", shown(&new_member)),
            );
            once(written, &new_member, "成员")?;
            plan.write(key, "SREM", "srem", &[&member]);
            plan.write(key, "SADD", "sadd", &[&new_member]);
        }
        RowChange::Delete { key: row } => {
            let member = original(row, "member")?;
            plan.check(
                "is_member",
                member.clone(),
                "",
                format!("成员 {}", shown(&member)),
            );
            plan.write(key, "SREM", "srem", &[&member]);
        }
        RowChange::Insert { values } => {
            let member = required(values, "member", "成员")?;
            once(written, &member, "成员")?;
            plan.check(
                "no_member",
                member.clone(),
                "",
                format!("成员 {}", shown(&member)),
            );
            plan.write(key, "SADD", "sadd", &[&member]);
        }
    }
    Ok(())
}

fn zset_change(
    plan: &mut Plan,
    key: &[u8],
    change: &RowChange,
    written: &mut BTreeSet<Vec<u8>>,
) -> CoreResult<()> {
    match change {
        RowChange::Update { key: row, values } => {
            let (member, old_score) = (original(row, "member")?, original(row, "score")?);
            let new_member = edited(values, "member")?.unwrap_or_else(|| member.clone());
            let new_score = match edited(values, "score")? {
                Some(raw) => score(raw)?,
                None => old_score.clone(),
            };
            plan.check(
                "eq_zscore",
                member.clone(),
                old_score,
                format!("成员 {}", shown(&member)),
            );
            once(written, &new_member, "成员")?;
            if new_member != member {
                plan.check(
                    "no_zmember",
                    new_member.clone(),
                    "",
                    format!("成员 {}", shown(&new_member)),
                );
                plan.write(key, "ZREM", "zrem", &[&member]);
            }
            plan.write(key, "ZADD", "zadd", &[&new_score, &new_member]);
        }
        RowChange::Delete { key: row } => {
            let (member, old_score) = (original(row, "member")?, original(row, "score")?);
            plan.check(
                "eq_zscore",
                member.clone(),
                old_score,
                format!("成员 {}", shown(&member)),
            );
            plan.write(key, "ZREM", "zrem", &[&member]);
        }
        RowChange::Insert { values } => {
            let member = required(values, "member", "成员")?;
            let new_score = score(required(values, "score", "分数")?)?;
            once(written, &member, "成员")?;
            plan.check(
                "no_zmember",
                member.clone(),
                "",
                format!("成员 {}", shown(&member)),
            );
            plan.write(key, "ZADD", "zadd", &[&new_score, &member]);
        }
    }
    Ok(())
}

/// 新建一个键。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewKey {
    pub name: String,
    /// string、hash、list、set、zset、stream。
    pub kind: String,
    /// 过期秒数;不过期为 `None`。
    pub ttl_seconds: Option<u64>,
    /// hash、stream 的第一个字段。
    pub field: Option<String>,
    /// 字符串的值,集合类的第一个元素 / 成员,hash 和 stream 的第一个值。
    pub value: String,
    /// zset 的第一个分数。
    pub score: Option<String>,
}

fn create_plan(new: &NewKey) -> CoreResult<Plan> {
    let key = new.name.as_bytes();
    if key.is_empty() {
        return Err(CoreError::InvalidEdit("请填写键名".into()));
    }
    let mut plan = Plan {
        steps: vec![Step::new("absent", "", "", String::new())],
        commands: Vec::new(),
    };
    let value = new.value.as_bytes();
    let field = || {
        new.field
            .as_deref()
            .filter(|f| !f.is_empty())
            .map(str::as_bytes)
            .ok_or_else(|| CoreError::InvalidEdit("请填写第一个字段".into()))
    };
    match new.kind.as_str() {
        "string" => {
            plan.commands.push(command_text("SET", key, &[value]));
            plan.steps
                .push(Step::write("set", value.to_vec(), Vec::new()));
        }
        "hash" => plan.write(key, "HSET", "hset", &[field()?, value]),
        "list" => plan.write(key, "RPUSH", "rpush", &[value]),
        "set" => plan.write(key, "SADD", "sadd", &[value]),
        "zset" => {
            let raw = new.score.clone().unwrap_or_default().into_bytes();
            plan.write(key, "ZADD", "zadd", &[&score(raw)?, value]);
        }
        "stream" => {
            let field = field()?;
            plan.commands
                .push(command_text("XADD", key, &[b"*", field, value]));
            plan.steps
                .push(Step::write("xadd", field.to_vec(), value.to_vec()));
        }
        other => return Err(CoreError::InvalidEdit(format!("不能新建 {other} 类型的键"))),
    }
    if let Some(ttl) = new.ttl_seconds {
        check_ttl(ttl)?;
        let seconds = ttl.to_string();
        plan.write(key, "EXPIRE", "expire", &[seconds.as_bytes()]);
    }
    Ok(plan)
}

impl RedisSession {
    /// 给人看的命令,不执行。
    pub fn preview_key_changes(
        &self,
        id: &str,
        kind: &str,
        changes: &[RowChange],
    ) -> CoreResult<Vec<String>> {
        Ok(plan(kind, &key_bytes(id)?, changes)?.commands)
    }

    /// 核对后一起写入。`expected` 是用户看过的预览:重新生成的不一样就不执行。
    pub async fn apply_key_changes(
        &self,
        db: u32,
        id: &str,
        kind: &str,
        changes: &[RowChange],
        expected: Option<&[String]>,
    ) -> CoreResult<ApplyOutcome> {
        let key = key_bytes(id)?;
        let plan = plan(kind, &key, changes)?;
        if expected.is_some_and(|expected| expected != plan.commands.as_slice()) {
            return Err(CoreError::InvalidEdit(
                "要执行的命令和预览时不一样了，请重新预览".into(),
            ));
        }
        let statements = plan.commands.len();
        self.run_plan(db, &key, plan).await?;
        Ok(ApplyOutcome {
            statements,
            affected_rows: changes.len() as u64,
        })
    }

    /// 改字符串的值:`original` 是读出来时的值,被别人改过就不覆盖。剩余时间不变。
    pub async fn set_string(
        &self,
        db: u32,
        id: &str,
        original: &str,
        value: &str,
    ) -> CoreResult<()> {
        let key = key_bytes(id)?;
        let mut plan = Plan::new("string");
        plan.check("eq_get", original, "", "这个值".into());
        plan.commands
            .push(command_text("SET", &key, &[value.as_bytes()]));
        plan.steps
            .push(Step::write("set", value.as_bytes().to_vec(), Vec::new()));
        self.run_plan(db, &key, plan).await
    }

    pub async fn create_key(&self, db: u32, new: &NewKey) -> CoreResult<KeyName> {
        let plan = create_plan(new)?;
        self.run_plan(db, new.name.as_bytes(), plan).await?;
        Ok(KeyName::from_bytes(new.name.as_bytes()))
    }

    /// 改名;新名字已经有键时不覆盖。
    pub async fn rename_key(&self, db: u32, id: &str, new_name: &str) -> CoreResult<KeyName> {
        let key = key_bytes(id)?;
        if new_name.is_empty() {
            return Err(CoreError::InvalidEdit("请填写新的键名".into()));
        }
        let renamed: i64 = self
            .execute(db, cmd("RENAMENX").arg(&key).arg(new_name))
            .await
            .map_err(|error| match error {
                CoreError::Database(message) if message.contains("no such key") => {
                    CoreError::NotFound("这个键已经不存在了".into())
                }
                other => other,
            })?;
        if renamed == 0 {
            return Err(CoreError::AlreadyExists(format!(
                "已经有叫 {} 的键了，没有改名",
                quote(new_name.as_bytes())
            )));
        }
        Ok(KeyName::from_bytes(new_name.as_bytes()))
    }

    /// 删除;大键用 UNLINK 在后台释放,不卡服务器(Redis 4 之前没有 UNLINK,用 DEL)。返回删掉了几个。
    pub async fn delete_keys(&self, db: u32, ids: &[String]) -> CoreResult<u64> {
        let keys = ids
            .iter()
            .map(|id| key_bytes(id))
            .collect::<CoreResult<Vec<_>>>()?;
        if keys.is_empty() {
            return Ok(0);
        }
        match self.execute(db, cmd("UNLINK").arg(&keys)).await {
            Err(CoreError::Database(message)) if message.contains("unknown command") => {
                self.execute(db, cmd("DEL").arg(&keys)).await
            }
            other => other,
        }
    }

    /// 设过期时间;`None` 是不过期。
    pub async fn set_ttl(&self, db: u32, id: &str, seconds: Option<u64>) -> CoreResult<()> {
        let key = key_bytes(id)?;
        let done: i64 = match seconds {
            Some(seconds) => {
                check_ttl(seconds)?;
                self.execute(db, cmd("EXPIRE").arg(&key).arg(seconds))
                    .await?
            }
            None => {
                // 本来就不过期时 PERSIST 也回 0:先看键在不在。
                let exists: i64 = self.query(db, cmd("EXISTS").arg(&key)).await?;
                if exists == 1 {
                    let _: i64 = self.execute(db, cmd("PERSIST").arg(&key)).await?;
                }
                exists
            }
        };
        if done == 0 {
            return Err(CoreError::NotFound("这个键已经不存在了".into()));
        }
        Ok(())
    }

    async fn run_plan(&self, db: u32, key: &[u8], plan: Plan) -> CoreResult<()> {
        let script = Script::new(SCRIPT);
        let mut invocation = script.key(key);
        for step in &plan.steps {
            invocation.arg(step.op).arg(&step.a).arg(&step.b);
        }
        match self.invoke(db, &invocation).await {
            Ok(()) => Ok(()),
            Err(CoreError::Database(message)) => Err(explain(&message, &plan)),
            Err(other) => Err(other),
        }
    }
}

/// 脚本的核对失败「BASALT 第几步 原因」换成人话。
fn explain(message: &str, plan: &Plan) -> CoreError {
    let Some(rest) = message.split("BASALT").nth(1) else {
        if message.contains("NOSCRIPT")
            || message.contains("NOPERM")
            || message.contains("unknown command")
        {
            return CoreError::Database(format!(
                "这个账号不能执行 Lua 脚本，Basalt 没法安全地提交修改（{message}）。可以在命令行里直接执行命令"
            ));
        }
        return CoreError::Database(message.to_owned());
    };
    let mut parts = rest.split_whitespace();
    let step = parts
        .next()
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let what = plan
        .steps
        .get(step.saturating_sub(1))
        .map(|s| s.what.as_str())
        .unwrap_or("这一项");
    match parts.next() {
        Some("type") => CoreError::RowMismatch(
            "这个键已经不存在了（过期或被删），或者变成了别的类型，这批修改没有提交。请刷新".into(),
        ),
        Some("exists") if plan.steps.first().is_some_and(|s| s.op == "absent") => {
            CoreError::AlreadyExists("已经有同名的键了，没有新建".into())
        }
        Some("exists") => CoreError::RowMismatch(format!("{what}已经有了，这批修改没有提交")),
        _ => CoreError::RowMismatch(format!(
            "{what}已经被别人改过或删掉了，这批修改没有提交。请刷新后再改"
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn row(pairs: &[(&str, CellValue)]) -> RowKey {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn values(pairs: &[(&str, &str)]) -> EditValues {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Some((*v).to_owned())))
            .collect()
    }

    fn text(value: &str) -> CellValue {
        CellValue::Text(value.into())
    }

    #[test]
    fn quotes_like_redis_cli() {
        assert_eq!(quote(b"user:1001"), "user:1001");
        assert_eq!(quote("黄勇".as_bytes()), "黄勇");
        assert_eq!(quote(b"two words"), "\"two words\"");
        assert_eq!(quote(b""), "\"\"");
        assert_eq!(quote(b"say \"hi\"\n"), "\"say \\\"hi\\\"\\n\"");
        assert_eq!(quote(&[0x00, 0xff, b'a']), "\"\\x00\\xffa\"");
        assert_eq!(quote("a\u{85}".as_bytes()), "\"a\\xc2\\x85\"");
        assert_eq!(quote("x\u{202e}y".as_bytes()), "\"x\\xe2\\x80\\xaey\"");
        assert_eq!(quote("\u{feff}k".as_bytes()), "\"\\xef\\xbb\\xbfk\"");
    }

    #[test]
    fn hash_edits_check_the_old_value_and_rename_fields_safely() {
        let changes = vec![
            RowChange::Update {
                key: row(&[("field", text("name")), ("value", text("黄勇"))]),
                values: values(&[("value", "黄 勇")]),
            },
            RowChange::Update {
                key: row(&[("field", text("city")), ("value", text("杭州"))]),
                values: values(&[("field", "home")]),
            },
            RowChange::Delete {
                key: row(&[("field", text("vip")), ("value", text("0"))]),
            },
            RowChange::Insert {
                values: values(&[("field", "email"), ("value", "a@b.c")]),
            },
        ];
        let plan = plan("hash", b"user:1001", &changes).unwrap();
        assert_eq!(
            plan.commands,
            [
                "HSET user:1001 name \"黄 勇\"",
                "HDEL user:1001 city",
                "HSET user:1001 home 杭州",
                "HDEL user:1001 vip",
                "HSET user:1001 email a@b.c",
            ]
        );
        let ops: Vec<&str> = plan.steps.iter().map(|s| s.op).collect();
        assert_eq!(
            ops,
            [
                "type",
                "eq_hget",
                "hset",
                "eq_hget",
                "no_hfield",
                "hdel",
                "hset",
                "eq_hget",
                "hdel",
                "no_hfield",
                "hset"
            ]
        );
    }

    #[test]
    fn a_field_written_twice_in_one_batch_is_refused() {
        let changes = vec![
            RowChange::Insert {
                values: values(&[("field", "a"), ("value", "1")]),
            },
            RowChange::Insert {
                values: values(&[("field", "a"), ("value", "2")]),
            },
        ];
        let error = plan("hash", b"h", &changes).err().unwrap();
        assert!(error.to_string().contains("出现了两次"), "{error}");
    }

    #[test]
    fn list_deletes_mark_then_remove_once() {
        let index = |i: i64| CellValue::Int(i);
        let changes = vec![
            RowChange::Update {
                key: row(&[("index", index(0)), ("value", text("a"))]),
                values: values(&[("value", "A")]),
            },
            RowChange::Delete {
                key: row(&[("index", index(2)), ("value", text("c"))]),
            },
            RowChange::Insert {
                values: values(&[("value", "z")]),
            },
        ];
        let plan = plan("list", b"l", &changes).unwrap();
        assert_eq!(
            plan.commands,
            [
                "LSET l 0 A",
                "LSET l 2 \"\\x00basalt:deleted\"",
                "RPUSH l z",
                "LREM l 0 \"\\x00basalt:deleted\""
            ]
        );
    }

    #[test]
    fn zset_scores_must_be_numbers() {
        let insert = |score: &str| RowChange::Insert {
            values: values(&[("member", "m"), ("score", score)]),
        };
        for ok in ["1.5", "-inf", "+inf", "0", "0.0e9", "-2e-300", "1E5", " 7 "] {
            assert!(plan("zset", b"z", &[insert(ok)]).is_ok(), "{ok}");
        }
        // 写入阶段出错脚本不会回滚:Redis 不认的(溢出成无穷、下溢成 0、NaN)在这里就要拦下。
        for bad in [
            "abc", "nan", "infinity", "1e400", "-1e400", "1e-400", "0x10", "",
        ] {
            let error = plan("zset", b"z", &[insert(bad)]).err().unwrap();
            assert!(
                error.to_string().contains("不是 Redis 能用的数"),
                "{bad}: {error}"
            );
        }
    }

    #[test]
    fn expiry_is_between_one_second_and_a_hundred_years() {
        let key = |ttl| NewKey {
            name: "k".into(),
            kind: "string".into(),
            ttl_seconds: Some(ttl),
            field: None,
            value: "v".into(),
            score: None,
        };
        assert!(create_plan(&key(1)).is_ok());
        assert!(create_plan(&key(MAX_TTL_SECONDS)).is_ok());
        assert!(create_plan(&key(0)).is_err());
        assert!(create_plan(&key(MAX_TTL_SECONDS + 1)).is_err());
    }

    #[test]
    fn redis_has_no_null() {
        let change = RowChange::Update {
            key: row(&[("member", text("m"))]),
            values: BTreeMap::from([("member".to_owned(), None)]),
        };
        assert!(plan("set", b"s", &[change]).is_err());
    }

    #[test]
    fn script_failures_name_the_item_that_changed() {
        let changes = vec![RowChange::Delete {
            key: row(&[("field", text("vip")), ("value", text("0"))]),
        }];
        let plan = plan("hash", b"h", &changes).unwrap();
        let error = explain("BASALT 2 changed", &plan);
        assert_eq!(error.code(), "row_mismatch");
        assert!(
            error.to_string().contains("字段 vip已经被别人改过"),
            "{error}"
        );
        assert!(
            explain("BASALT 1 type", &plan)
                .to_string()
                .contains("不存在")
        );
    }

    #[test]
    fn new_keys_need_what_their_type_needs() {
        let new = |kind: &str, field: Option<&str>, score: Option<&str>| NewKey {
            name: "k".into(),
            kind: kind.into(),
            ttl_seconds: Some(60),
            field: field.map(Into::into),
            value: "v".into(),
            score: score.map(Into::into),
        };
        assert_eq!(
            create_plan(&new("string", None, None)).unwrap().commands,
            ["SET k v", "EXPIRE k 60"]
        );
        assert!(create_plan(&new("hash", None, None)).is_err());
        assert_eq!(
            create_plan(&new("hash", Some("f"), None)).unwrap().commands[0],
            "HSET k f v"
        );
        assert!(create_plan(&new("zset", None, Some("x"))).is_err());
        assert_eq!(
            create_plan(&new("stream", Some("f"), None))
                .unwrap()
                .commands[0],
            "XADD k * f v"
        );
    }
}
