//! 命令行:每个命令行标签页一条自己的连接,`SELECT`、`MULTI`、`WATCH` 只对它生效。
//! 命令不设超时(`BLPOP` 可以一直等),要停就断开这条连接,下一条命令再连。
//! 连接自己断了(服务器踢掉空闲连接、网络切换)而事务、WATCH 还在时不悄悄重连:报错一次,让用户知道。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use redis::aio::ConnectionLike;
use redis::{Cmd, Value};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

use super::RedisSession;
use super::link::Live;
use crate::error::{CoreError, CoreResult};

/// 数组最多带回这么多项,字符串最多这么多字节;再多界面也看不过来。
const MAX_ITEMS: usize = 10_000;
const MAX_BYTES: usize = 1 << 20;
/// 一条回复(嵌套的全算上)最多带回这么多项、这么多字节,免得一次传给界面几个 G。
const MAX_TOTAL_ITEMS: usize = 100_000;
const MAX_TOTAL_BYTES: usize = 8 << 20;

/// 一条命令的回复,结构照 RESP 原样保留,前端照 redis-cli 的样子显示。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Reply {
    Nil,
    Status {
        text: String,
    },
    Error {
        text: String,
    },
    /// 整数按字符串传:64 位整数在前端的 number 里会丢精度。
    Int {
        value: String,
    },
    /// RESP3 的浮点数、大数;按服务器给的写法原样显示。
    Number {
        text: String,
    },
    Bool {
        value: bool,
    },
    /// 字符串。不是 UTF-8 时 `text` 是 redis-cli 那样的 `\xHH` 转义,`binary` 为真。
    #[serde(rename_all = "camelCase")]
    Bulk {
        text: String,
        binary: bool,
        /// 原文有多少字节。
        bytes: usize,
        /// 太长,只带回了前面一段。
        truncated: bool,
    },
    Array {
        items: Vec<Reply>,
        /// 一共有几项;比 `items` 多时后面的没带回来。
        total: usize,
    },
    Map {
        entries: Vec<(Reply, Reply)>,
        total: usize,
    },
}

/// 跑一条命令的结果。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum ConsoleOutcome {
    /// 执行了。`db` 是执行后这条连接所在的库(`SELECT` 之后会变),`transaction` 表示在 `MULTI` 里。
    #[serde(rename_all = "camelCase")]
    Ran {
        reply: Reply,
        db: u32,
        transaction: bool,
        elapsed_ms: u64,
    },
    /// 危险的命令,没有执行;确认之后带上 `confirmed` 再发一次。
    Confirm { reason: String },
}

/// 一个命令行标签页的连接。`abort` 单独放着:命令在跑时 `state` 被占着,也能断开它。
pub(super) struct Console {
    state: Mutex<Option<State>>,
    abort: std::sync::Mutex<Option<AbortHandle>>,
    /// 用户停下过:下一条命令换新连接(停下时已经告诉过用户事务没了,不再报错)。
    /// 收发任务停下要等一会儿,不能只看它停没停。
    dropped: AtomicBool,
    /// 标签页关了:还在路上的命令不再执行。
    closed: AtomicBool,
}

struct State {
    live: Live,
    db: u32,
    transaction: bool,
    /// `MULTI` 里排队的命令,是 `SELECT n` 的记下 n:`EXEC` 之后照各自的回复换库。
    queued: Vec<Option<u32>>,
    /// `WATCH` 过,还没 `EXEC`、`DISCARD`、`UNWATCH`。
    watching: bool,
    /// 用 `AUTH` 换过账号:重连会回到连接配置里的账号。
    switched_user: bool,
}

impl State {
    fn new(live: Live, db: u32) -> Self {
        Self {
            live,
            db,
            transaction: false,
            queued: Vec::new(),
            watching: false,
            switched_user: false,
        }
    }

    /// 连接断了就会丢掉的东西;有的话重连之前要先告诉用户。
    fn fragile(&self) -> Option<&'static str> {
        if self.transaction {
            Some("事务")
        } else if self.watching {
            Some("WATCH")
        } else if self.switched_user {
            Some("用 AUTH 换的账号")
        } else {
            None
        }
    }
}

/// 换下来的连接马上断开,不留在服务器上。
impl Drop for State {
    fn drop(&mut self) {
        self.live.driver.abort();
    }
}

impl Console {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(None),
            abort: std::sync::Mutex::new(None),
            dropped: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        })
    }

    /// 断开连接:正在跑的命令立刻失败,下一条命令重新连。
    pub(super) fn disconnect(&self) {
        self.dropped.store(true, Ordering::SeqCst);
        if let Some(abort) = self.abort.lock().unwrap_or_else(|e| e.into_inner()).take() {
            abort.abort();
        }
    }

    /// 标签页关了:断开,之后到的命令一律不执行。
    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.disconnect();
    }
}

impl RedisSession {
    /// 在命令行标签页 `console` 里执行一行命令。`db` 是界面上显示的库号,连接断过之后按它重连。
    pub async fn run_console(
        &self,
        console: &str,
        db: u32,
        line: &str,
        confirmed: bool,
    ) -> CoreResult<ConsoleOutcome> {
        let args = split_args(line).map_err(CoreError::InvalidEdit)?;
        let Some(name) = args.first() else {
            return Err(CoreError::InvalidEdit("请输入命令".into()));
        };
        let verb = command_name(name, args.get(1));
        if let Some(reason) = refused(&verb) {
            return Err(CoreError::Unsupported(reason));
        }
        if !confirmed && let Some(reason) = dangerous(&verb) {
            return Ok(ConsoleOutcome::Confirm { reason });
        }

        let console = self.console(console).await?;
        let mut slot = console.state.lock().await;
        if console.closed.load(Ordering::SeqCst) {
            return Err(CoreError::Cancelled);
        }
        let dropped = console.dropped.swap(false, Ordering::SeqCst);
        let alive = slot.as_ref().is_some_and(|s| !s.live.driver.is_finished());
        if !dropped
            && !alive
            && let Some(lost) = slot.as_ref().and_then(State::fragile)
        {
            *slot = None;
            return Err(CoreError::Network(format!(
                "命令行的连接断开了，{lost}已经失效。这条命令没有执行；再执行一次会重新连接"
            )));
        }
        if dropped || !alive {
            let live = self.link.connect(db, None).await?;
            *console.abort.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(live.driver.abort_handle());
            *slot = Some(State::new(live, db));
            // 连接的时候用户按了停止,或者标签页、连接关了:这条不执行。
            if console.dropped.swap(false, Ordering::SeqCst)
                || console.closed.load(Ordering::SeqCst)
                || self.closed.load(Ordering::SeqCst)
            {
                *slot = None;
                return Err(CoreError::Cancelled);
            }
        }
        let state = slot.as_mut().expect("刚连上");

        let mut cmd = Cmd::new();
        for arg in &args {
            cmd.arg(arg.as_slice());
        }
        let started = Instant::now();
        let value = match state.live.conn.req_packed_command(&cmd).await {
            Ok(value) => value,
            Err(error) => {
                // 断开了(或者被用户停下):换掉这条连接,事务和 WATCH 都没了。
                *slot = None;
                return Err(if error.is_io_error() || error.is_connection_dropped() {
                    CoreError::Network(
                        "命令行的连接断开了，下一条命令会重新连接（事务、WATCH 和用 AUTH 换的账号都已经失效）"
                            .into(),
                    )
                } else {
                    CoreError::from(error)
                });
            }
        };
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let reply = Reply::from(value);
        follow(state, &verb, &args, &reply);
        Ok(ConsoleOutcome::Ran {
            reply,
            db: state.db,
            transaction: state.transaction,
            elapsed_ms,
        })
    }

    /// 停下正在跑的命令(断开连接)。
    pub async fn cancel_console(&self, console: &str) {
        if let Some(console) = self.consoles.lock().await.get(console) {
            console.disconnect();
        }
    }

    /// 标签页关了:断开并忘掉这条连接。
    pub async fn close_console(&self, console: &str) {
        if let Some(console) = self.consoles.lock().await.remove(console) {
            console.close();
        }
    }

    /// 服务器认识的命令名(补全用)。`COMMAND LIST` 要 Redis 7,老版本从 `COMMAND` 里取。
    pub async fn command_names(&self) -> CoreResult<Vec<String>> {
        let db = self.default_db;
        let mut names: Vec<String> = match self.query(db, redis::cmd("COMMAND").arg("LIST")).await {
            Ok(names) => names,
            Err(_) => {
                let all: Vec<Value> = self.query(db, &redis::cmd("COMMAND")).await?;
                all.into_iter()
                    .filter_map(|entry| match entry {
                        Value::Array(fields) => match fields.into_iter().next() {
                            Some(Value::BulkString(name)) => String::from_utf8(name).ok(),
                            Some(Value::SimpleString(name)) => Some(name),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect()
            }
        };
        for name in &mut names {
            name.make_ascii_uppercase();
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    async fn console(&self, id: &str) -> CoreResult<Arc<Console>> {
        let mut consoles = self.consoles.lock().await;
        // 会话已经关了(和 close 交错):不再开新的命令行连接。
        if self.closed.load(Ordering::SeqCst) {
            return Err(CoreError::Network("连接已经关闭".into()));
        }
        Ok(consoles
            .entry(id.to_owned())
            .or_insert_with(Console::new)
            .clone())
    }
}

/// 执行之后记下连接状态的变化:换了库、进出事务、WATCH、换了账号。
/// 事务里的命令只是排队(回 `QUEUED`),`EXEC` 之后才照各自的回复算数。
fn follow(state: &mut State, verb: &str, args: &[Vec<u8>], reply: &Reply) {
    let status = |text: &str| matches!(reply, Reply::Status { text: t } if t == text);
    let selected = || {
        args.get(1)
            .and_then(|a| std::str::from_utf8(a).ok())
            .and_then(|a| a.parse().ok())
    };
    if state.transaction {
        match verb {
            // EXEC 出错(EXECABORT)、WATCH 的键被改过(回 nil)事务也都结束了,只是什么都没执行。
            "EXEC" | "DISCARD" => {
                let queued = std::mem::take(&mut state.queued);
                state.transaction = false;
                state.watching = false;
                if let Reply::Array { items, .. } = reply {
                    for (select, item) in queued.iter().zip(items) {
                        if let (Some(db), Reply::Status { text }) = (select, item)
                            && text == "OK"
                        {
                            state.db = *db;
                        }
                    }
                }
            }
            _ if status("QUEUED") => {
                state
                    .queued
                    .push(if verb == "SELECT" { selected() } else { None });
            }
            _ => {}
        }
        return;
    }
    match verb {
        "SELECT" if status("OK") => {
            if let Some(db) = selected() {
                state.db = db;
            }
        }
        "MULTI" if status("OK") => state.transaction = true,
        "WATCH" if status("OK") => state.watching = true,
        "UNWATCH" if status("OK") => state.watching = false,
        "AUTH" if status("OK") => state.switched_user = true,
        _ => {}
    }
}

/// 命令名大写;`CONFIG SET` 这类带子命令的连子命令一起。
fn command_name(name: &[u8], sub: Option<&Vec<u8>>) -> String {
    let name = String::from_utf8_lossy(name).to_ascii_uppercase();
    const WITH_SUB: &[&str] = &[
        "CONFIG", "CLIENT", "SCRIPT", "FUNCTION", "CLUSTER", "ACL", "DEBUG", "MEMORY", "OBJECT",
        "SLOWLOG", "LATENCY", "MODULE", "XINFO", "XGROUP", "COMMAND",
    ];
    match sub {
        Some(sub) if WITH_SUB.contains(&name.as_str()) => {
            format!(
                "{name} {}",
                String::from_utf8_lossy(sub).to_ascii_uppercase()
            )
        }
        _ => name,
    }
}

/// 命令行做不了的命令:会把连接变成只收推送,或者换了协议,之后的回复对不上号。
fn refused(verb: &str) -> Option<String> {
    let first = verb.split(' ').next().unwrap_or(verb);
    let reason = match first {
        "SUBSCRIBE" | "PSUBSCRIBE" | "SSUBSCRIBE" | "UNSUBSCRIBE" | "PUNSUBSCRIBE"
        | "SUNSUBSCRIBE" => {
            "命令行暂不支持订阅（SUBSCRIBE 之后连接只收推送消息）。发布用 PUBLISH 可以"
        }
        "MONITOR" => "命令行暂不支持 MONITOR（它会一直推送服务器收到的每条命令）",
        "SYNC" | "PSYNC" | "REPLCONF" => "这是主从复制内部用的命令，命令行不支持",
        "HELLO" => "命令行固定用 RESP2 协议，不支持 HELLO 切换协议",
        "QUIT" | "RESET" => "关掉这个标签页就断开了；要换库用 SELECT",
        _ if verb == "CLIENT REPLY" => "CLIENT REPLY 会让之后的命令收不到回复，命令行不支持",
        _ => return None,
    };
    Some(reason.into())
}

/// 要先确认才执行的命令:删整库、停服务器、阻塞整个服务器,或者改了就很难改回来。
fn dangerous(verb: &str) -> Option<String> {
    let reason = match verb {
        "FLUSHALL" => "FLUSHALL 会删掉所有库里的全部键，不能恢复",
        "FLUSHDB" => "FLUSHDB 会删掉当前库里的全部键，不能恢复",
        "SHUTDOWN" => "SHUTDOWN 会关掉 Redis 服务器，所有客户端都会断开",
        "KEYS" => {
            "KEYS 会一次遍历整个库，键多时会卡住服务器、影响别的客户端。浏览键可以用键列表或 SCAN"
        }
        "SWAPDB" => "SWAPDB 会把两个库的数据整个对调，所有客户端立刻看到变化",
        "SCRIPT FLUSH" => "SCRIPT FLUSH 会清掉服务器上缓存的所有 Lua 脚本",
        "FUNCTION FLUSH" => "FUNCTION FLUSH 会删掉服务器上所有的函数库",
        "FUNCTION DELETE" => "FUNCTION DELETE 会删掉一个函数库",
        "CLIENT KILL" => "CLIENT KILL 会断开别的客户端的连接",
        "CONFIG SET" => "CONFIG SET 会立刻改服务器的运行配置，影响所有客户端",
        "CONFIG RESETSTAT" => "CONFIG RESETSTAT 会把服务器的统计数据清零",
        "REPLICAOF" | "SLAVEOF" => {
            "这会改变主从关系：变成从节点后，这台服务器上现有的数据会被主节点的数据替换"
        }
        "FAILOVER" => "FAILOVER 会把主节点切换到从节点",
        "ACL SETUSER" | "ACL DELUSER" | "ACL LOAD" => {
            "这会改 Redis 账号的权限，改错了可能把自己或别人关在门外"
        }
        "DEBUG" | "MODULE LOAD" | "MODULE LOADEX" | "MODULE UNLOAD" => {
            "这条命令会直接影响服务器进程"
        }
        "CLIENT PAUSE" => "CLIENT PAUSE 会让所有客户端的命令都停下来等着，直到时间到",
        "FUNCTION RESTORE" => "FUNCTION RESTORE 会覆盖服务器上的函数库（带 FLUSH 时先全部删掉）",
        "SAVE" => "SAVE 会在前台写快照，写完之前服务器不处理别的命令；平时用 BGSAVE",
        "CONFIG REWRITE" => "CONFIG REWRITE 会把现在的运行配置写回服务器的配置文件",
        _ if verb.starts_with("DEBUG ") => "DEBUG 命令可能让服务器卡住或崩溃",
        _ => return None,
    };
    Some(reason.into())
}

/// 按 redis-cli 的规则把一行拆成参数:空白分隔;双引号里认 `\n` `\t` `\xHH` 这类转义,
/// 单引号里只认 `\'`;引号收尾后面必须是空白或行尾。
pub fn split_args(line: &str) -> Result<Vec<Vec<u8>>, String> {
    let bytes = line.as_bytes();
    let mut args = Vec::new();
    let mut i = 0;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return Ok(args);
        }
        let mut arg = Vec::new();
        loop {
            match bytes.get(i) {
                None => break,
                Some(b) if b.is_ascii_whitespace() => break,
                Some(b'"') => i = quoted(bytes, i + 1, b'"', &mut arg)?,
                Some(b'\'') => i = quoted(bytes, i + 1, b'\'', &mut arg)?,
                Some(&b) => {
                    arg.push(b);
                    i += 1;
                }
            }
        }
        args.push(arg);
    }
}

/// 引号里的一段,从开引号后面读到收尾的引号;返回收尾引号后面的位置。
fn quoted(bytes: &[u8], mut i: usize, quote: u8, out: &mut Vec<u8>) -> Result<usize, String> {
    loop {
        let Some(&b) = bytes.get(i) else {
            return Err("引号没有配对".into());
        };
        if b == quote {
            return match bytes.get(i + 1) {
                None => Ok(i + 1),
                Some(next) if next.is_ascii_whitespace() => Ok(i + 1),
                Some(_) => Err("引号收尾后面要有空格".into()),
            };
        }
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if quote == b'\'' {
                if next == b'\'' {
                    out.push(b'\'');
                    i += 2;
                    continue;
                }
            } else {
                let hex = |b: u8| (b as char).to_digit(16);
                if next == b'x'
                    && let (Some(h), Some(l)) = (
                        bytes.get(i + 2).and_then(|&b| hex(b)),
                        bytes.get(i + 3).and_then(|&b| hex(b)),
                    )
                {
                    out.push((h * 16 + l) as u8);
                    i += 4;
                    continue;
                }
                let escaped = match next {
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'b' => 0x08,
                    b'a' => 0x07,
                    other => other,
                };
                out.push(escaped);
                i += 2;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
}

impl From<Value> for Reply {
    fn from(value: Value) -> Self {
        convert(
            value,
            &mut Budget {
                items: MAX_TOTAL_ITEMS,
                bytes: MAX_TOTAL_BYTES,
            },
        )
    }
}

/// 一条回复还能带回多少项、多少字节。
struct Budget {
    items: usize,
    bytes: usize,
}

fn convert(value: Value, budget: &mut Budget) -> Reply {
    match value {
        Value::Nil => Reply::Nil,
        Value::Int(value) => Reply::Int {
            value: value.to_string(),
        },
        Value::BulkString(bytes) => bulk(bytes, budget),
        Value::SimpleString(text) => Reply::Status { text },
        Value::Okay => Reply::Status { text: "OK".into() },
        Value::Array(items) | Value::Set(items) => array(items, budget),
        Value::Push { data, .. } => array(data, budget),
        Value::Map(pairs) => {
            let total = pairs.len();
            let mut entries = Vec::new();
            for (k, v) in pairs.into_iter().take(MAX_ITEMS) {
                if budget.items == 0 {
                    break;
                }
                budget.items -= 1;
                entries.push((convert(k, budget), convert(v, budget)));
            }
            Reply::Map { entries, total }
        }
        Value::Attribute { data, .. } => convert(*data, budget),
        Value::Double(n) => Reply::Number {
            text: n.to_string(),
        },
        Value::Boolean(value) => Reply::Bool { value },
        Value::VerbatimString { text, .. } => bulk(text.into_bytes(), budget),
        Value::BigNumber(n) => Reply::Number {
            text: String::from_utf8_lossy(&n).into_owned(),
        },
        Value::ServerError(error) => Reply::Error {
            text: match error.details() {
                Some(details) => format!("{} {details}", error.code()),
                None => error.code().to_owned(),
            },
        },
        // `Value` 以后多了新的种类:照 Debug 的样子显示,不至于丢掉。
        #[allow(unreachable_patterns)]
        other => Reply::Status {
            text: format!("{other:?}"),
        },
    }
}

fn array(items: Vec<Value>, budget: &mut Budget) -> Reply {
    let total = items.len();
    let mut out = Vec::new();
    for item in items.into_iter().take(MAX_ITEMS) {
        if budget.items == 0 {
            break;
        }
        budget.items -= 1;
        out.push(convert(item, budget));
    }
    Reply::Array { items: out, total }
}

fn bulk(bytes: Vec<u8>, budget: &mut Budget) -> Reply {
    let total = bytes.len();
    let limit = total.min(MAX_BYTES).min(budget.bytes);
    let head = &bytes[..limit];
    budget.bytes -= limit;
    let truncated = limit < total;
    match std::str::from_utf8(head) {
        Ok(text) => Reply::Bulk {
            text: text.to_owned(),
            binary: false,
            bytes: total,
            truncated,
        },
        // 截断处正好切在一个字的中间:退回到这个字之前。
        Err(error) if error.error_len().is_none() && truncated => Reply::Bulk {
            text: String::from_utf8_lossy(&head[..error.valid_up_to()]).into_owned(),
            binary: false,
            bytes: total,
            truncated,
        },
        Err(_) => Reply::Bulk {
            text: escape_bytes(head),
            binary: true,
            bytes: total,
            truncated,
        },
    }
}

/// 不是 UTF-8 的字符串照 redis-cli 的样子写:可见 ASCII 原样,其余 `\xHH`。
fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        split_args(line)
            .unwrap()
            .into_iter()
            .map(|a| String::from_utf8(a).unwrap())
            .collect()
    }

    #[test]
    fn splits_like_redis_cli() {
        assert_eq!(args("  set  k   v "), ["set", "k", "v"]);
        assert_eq!(args(r#"set "a b" 'c d'"#), ["set", "a b", "c d"]);
        assert_eq!(
            args(r#"set k "line\none\t\"q\"""#),
            ["set", "k", "line\none\t\"q\""]
        );
        assert_eq!(args(r"set k 'it\'s \n'"), ["set", "k", "it's \\n"]);
        assert_eq!(args(r#"set k "\xe4\xbd\xa0好""#), ["set", "k", "你好"]);
        assert_eq!(
            args("hset 用户:1 名字 张三"),
            ["hset", "用户:1", "名字", "张三"]
        );
        assert_eq!(args(r#"set k """#), ["set", "k", ""]);
        assert_eq!(args(r#"get a"b c""#), ["get", "ab c"]);
        assert!(
            split_args(r#"get a"b c"d"#).is_err(),
            "和 redis-cli 一样:收尾引号后面紧跟字算错"
        );
        assert!(args("   ").is_empty());
        assert_eq!(split_args(r#"set k "open"#).unwrap_err(), "引号没有配对");
        assert_eq!(
            split_args(r#"set k "a"b"#).unwrap_err(),
            "引号收尾后面要有空格"
        );
        assert_eq!(
            split_args(r#"set "\x00\xff" v"#).unwrap()[1],
            vec![0u8, 0xff]
        );
    }

    #[test]
    fn knows_which_commands_need_a_second_thought() {
        let verb = |line: &str| {
            let a = split_args(line).unwrap();
            command_name(&a[0], a.get(1))
        };
        assert_eq!(verb("config set maxmemory 1gb"), "CONFIG SET");
        assert_eq!(verb("get config"), "GET");
        assert!(dangerous(&verb("flushall async")).is_some());
        assert!(dangerous(&verb("flushdb")).is_some());
        assert!(dangerous(&verb("keys *")).is_some());
        assert!(dangerous(&verb("Config Set maxmemory 1gb")).is_some());
        assert!(dangerous(&verb("config get maxmemory")).is_none());
        assert!(dangerous(&verb("debug sleep 10")).is_some());
        assert!(dangerous(&verb("get k")).is_none());
        for line in [
            "client pause 1000",
            "module loadex m",
            "function restore x flush",
            "save",
            "acl load",
            "config rewrite",
        ] {
            assert!(dangerous(&verb(line)).is_some(), "{line}");
        }
        assert!(dangerous(&verb("bgsave")).is_none());
        assert!(refused(&verb("subscribe news")).is_some());
        assert!(refused(&verb("MONITOR")).is_some());
        assert!(refused(&verb("hello 3")).is_some());
        assert!(refused(&verb("client reply off")).is_some());
        assert!(refused(&verb("client list")).is_none());
        assert!(refused(&verb("publish news hi")).is_none());
    }

    #[test]
    fn keeps_the_reply_shape() {
        let reply = Reply::from(Value::Array(vec![
            Value::BulkString(b"\xe4\xbd\xa0".to_vec()),
            Value::BulkString(vec![0, 0xff, b'a']),
            Value::Int(3),
            Value::Nil,
            Value::Okay,
        ]));
        assert_eq!(
            reply,
            Reply::Array {
                items: vec![
                    Reply::Bulk {
                        text: "你".into(),
                        binary: false,
                        bytes: 3,
                        truncated: false,
                    },
                    Reply::Bulk {
                        text: r"\x00\xffa".into(),
                        binary: true,
                        bytes: 3,
                        truncated: false,
                    },
                    Reply::Int { value: "3".into() },
                    Reply::Nil,
                    Reply::Status { text: "OK".into() },
                ],
                total: 5,
            }
        );
        let many = Reply::from(Value::Array(vec![Value::Int(1); MAX_ITEMS + 5]));
        assert!(
            matches!(many, Reply::Array { ref items, total } if items.len() == MAX_ITEMS && total == MAX_ITEMS + 5)
        );
    }

    #[test]
    fn caps_the_whole_reply_not_just_each_level() {
        let row = || Value::Array(vec![Value::Int(1); 1000]);
        let Reply::Array { items, total } = Reply::from(Value::Array(vec![row(); 1000])) else {
            panic!("应该是数组");
        };
        assert_eq!(total, 1000);
        let kept: usize = items
            .iter()
            .map(|r| match r {
                Reply::Array { items, .. } => items.len() + 1,
                _ => 1,
            })
            .sum();
        assert_eq!(kept, MAX_TOTAL_ITEMS);
        let big = || Value::BulkString(vec![b'a'; MAX_BYTES]);
        let Reply::Array { items, .. } = Reply::from(Value::Array(vec![big(); 10])) else {
            panic!("应该是数组");
        };
        let shown: usize = items
            .iter()
            .map(|r| match r {
                Reply::Bulk { text, .. } => text.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(shown, MAX_TOTAL_BYTES);
        assert!(matches!(
            items.last(),
            Some(Reply::Bulk {
                truncated: true,
                ..
            })
        ));
    }

    #[test]
    fn cuts_long_strings_on_a_character_boundary() {
        let mut text = "a".repeat(MAX_BYTES - 1);
        text.push('你');
        let Reply::Bulk {
            text: shown,
            binary,
            bytes,
            truncated,
        } = Reply::from(Value::BulkString(text.into_bytes()))
        else {
            panic!("应该是字符串");
        };
        assert!(!binary && truncated);
        assert_eq!(bytes, MAX_BYTES + 2);
        assert_eq!(shown.len(), MAX_BYTES - 1);
    }
}
