// 命令行一行怎么拆成参数、哪些命令要先确认或不支持。规则照着 crates/basalt-core/src/redis_db/console.rs,
// 后端是准;这里给 mock 后端和界面(历史里不记密码)用。

/**
 * 按 redis-cli 的规则把一行拆成参数:空白分隔;双引号里认 `\n` `\t` `\xHH` 这类转义,
 * 单引号里只认 `\'`;引号收尾后面必须是空白或行尾。拆不了时返回出错原因。
 */
export function splitArgs(line: string): string[] | { error: string } {
  const bytes = new TextEncoder().encode(line);
  const args: string[] = [];
  let i = 0;
  for (;;) {
    while (i < bytes.length && isSpace(bytes[i])) i += 1;
    if (i >= bytes.length) return args;
    const arg: number[] = [];
    while (i < bytes.length && !isSpace(bytes[i])) {
      const b = bytes[i]!;
      if (b === 0x22 || b === 0x27) {
        const end = quoted(bytes, i + 1, b, arg);
        if (typeof end === "string") return { error: end };
        i = end;
      } else {
        arg.push(b);
        i += 1;
      }
    }
    args.push(new TextDecoder().decode(new Uint8Array(arg)));
  }
}

const ESCAPES: Record<string, number> = { n: 0x0a, r: 0x0d, t: 0x09, b: 0x08, a: 0x07 };

/** 和 Rust 的 is_ascii_whitespace 一样:空格、\t、\n、\f、\r(不含 \v)。 */
const isSpace = (b: number | undefined) => b === 0x20 || b === 0x09 || b === 0x0a || b === 0x0c || b === 0x0d;

function quoted(bytes: Uint8Array, start: number, quote: number, out: number[]): number | string {
  let i = start;
  for (;;) {
    const b = bytes[i];
    if (b === undefined) return "引号没有配对";
    if (b === quote) return bytes[i + 1] === undefined || isSpace(bytes[i + 1]) ? i + 1 : "引号收尾后面要有空格";
    if (b === 0x5c && i + 1 < bytes.length) {
      const next = bytes[i + 1]!;
      if (quote === 0x27) {
        if (next === 0x27) {
          out.push(0x27);
          i += 2;
          continue;
        }
      } else {
        const hex = String.fromCharCode(bytes[i + 2] ?? 0, bytes[i + 3] ?? 0);
        if (next === 0x78 && /^[0-9a-f]{2}$/i.test(hex)) {
          out.push(parseInt(hex, 16));
          i += 4;
          continue;
        }
        out.push(ESCAPES[String.fromCharCode(next)] ?? next);
        i += 2;
        continue;
      }
    }
    out.push(b);
    i += 1;
  }
}

const WITH_SUB = new Set([
  "CONFIG",
  "CLIENT",
  "SCRIPT",
  "FUNCTION",
  "CLUSTER",
  "ACL",
  "DEBUG",
  "MEMORY",
  "OBJECT",
  "SLOWLOG",
  "LATENCY",
  "MODULE",
  "XINFO",
  "XGROUP",
  "COMMAND",
]);

/** 只把 ASCII 字母变大写(和 Redis、Rust 端一样;不按 Unicode 把 ß 变成 SS 这类)。 */
const asciiUpper = (text: string) => text.replace(/[a-z]/g, (c) => c.toUpperCase());

/** 命令名大写;`CONFIG SET` 这类带子命令的连子命令一起。 */
export function commandVerb(args: readonly string[]): string {
  const name = asciiUpper(args[0] ?? "");
  return WITH_SUB.has(name) && args[1] !== undefined ? `${name} ${asciiUpper(args[1])}` : name;
}

/** 命令行做不了的命令(会把连接变成只收推送,或者换了协议)。 */
export function refusedReason(verb: string): string | null {
  const first = verb.split(" ")[0]!;
  if (/^(P|S)?(UN)?SUBSCRIBE$/.test(first)) {
    return "命令行暂不支持订阅（SUBSCRIBE 之后连接只收推送消息）。发布用 PUBLISH 可以";
  }
  if (first === "MONITOR") return "命令行暂不支持 MONITOR（它会一直推送服务器收到的每条命令）";
  if (["SYNC", "PSYNC", "REPLCONF"].includes(first)) return "这是主从复制内部用的命令，命令行不支持";
  if (first === "HELLO") return "命令行固定用 RESP2 协议，不支持 HELLO 切换协议";
  if (first === "QUIT" || first === "RESET") return "关掉这个标签页就断开了；要换库用 SELECT";
  if (verb === "CLIENT REPLY") return "CLIENT REPLY 会让之后的命令收不到回复，命令行不支持";
  return null;
}

const DANGER: Record<string, string> = {
  FLUSHALL: "FLUSHALL 会删掉所有库里的全部键，不能恢复",
  FLUSHDB: "FLUSHDB 会删掉当前库里的全部键，不能恢复",
  SHUTDOWN: "SHUTDOWN 会关掉 Redis 服务器，所有客户端都会断开",
  KEYS: "KEYS 会一次遍历整个库，键多时会卡住服务器、影响别的客户端。浏览键可以用键列表或 SCAN",
  SWAPDB: "SWAPDB 会把两个库的数据整个对调，所有客户端立刻看到变化",
  "SCRIPT FLUSH": "SCRIPT FLUSH 会清掉服务器上缓存的所有 Lua 脚本",
  "FUNCTION FLUSH": "FUNCTION FLUSH 会删掉服务器上所有的函数库",
  "FUNCTION DELETE": "FUNCTION DELETE 会删掉一个函数库",
  "CLIENT KILL": "CLIENT KILL 会断开别的客户端的连接",
  "CONFIG SET": "CONFIG SET 会立刻改服务器的运行配置，影响所有客户端",
  "CONFIG RESETSTAT": "CONFIG RESETSTAT 会把服务器的统计数据清零",
  REPLICAOF: "这会改变主从关系：变成从节点后，这台服务器上现有的数据会被主节点的数据替换",
  SLAVEOF: "这会改变主从关系：变成从节点后，这台服务器上现有的数据会被主节点的数据替换",
  FAILOVER: "FAILOVER 会把主节点切换到从节点",
  "ACL SETUSER": "这会改 Redis 账号的权限，改错了可能把自己或别人关在门外",
  "ACL DELUSER": "这会改 Redis 账号的权限，改错了可能把自己或别人关在门外",
  "ACL LOAD": "这会改 Redis 账号的权限，改错了可能把自己或别人关在门外",
  DEBUG: "这条命令会直接影响服务器进程",
  "MODULE LOAD": "这条命令会直接影响服务器进程",
  "MODULE LOADEX": "这条命令会直接影响服务器进程",
  "MODULE UNLOAD": "这条命令会直接影响服务器进程",
  "CLIENT PAUSE": "CLIENT PAUSE 会让所有客户端的命令都停下来等着，直到时间到",
  "FUNCTION RESTORE": "FUNCTION RESTORE 会覆盖服务器上的函数库（带 FLUSH 时先全部删掉）",
  SAVE: "SAVE 会在前台写快照，写完之前服务器不处理别的命令；平时用 BGSAVE",
  "CONFIG REWRITE": "CONFIG REWRITE 会把现在的运行配置写回服务器的配置文件",
};

/** 要先确认才执行的命令,和为什么。 */
export function dangerReason(verb: string): string | null {
  return DANGER[verb] ?? (verb.startsWith("DEBUG ") ? "DEBUG 命令可能让服务器卡住或崩溃" : null);
}

/** 带密码的命令:历史里不记,回显时把密码遮住。 */
export function carriesSecret(args: readonly string[]): boolean {
  const verb = commandVerb(args);
  if (verb === "AUTH" || verb === "ACL SETUSER" || verb === "MIGRATE") return true;
  if (verb === "HELLO") return args.some((a) => asciiUpper(a) === "AUTH");
  return verb === "CONFIG SET" && args.some((a) => SECRET_CONFIG.test(a));
}

/** 配置项里放密码的那几个(Valkey 的 primaryauth 是 masterauth 的别名)。 */
const SECRET_CONFIG =
  /^(requirepass|masterauth|masteruser|primaryauth|primaryuser|tls-key-file-pass|tls-client-key-file-pass)$/i;

/** 拆不开(引号没配对)的一行:只看头一两个词像不像带密码的命令,宁可多遮。 */
export function mightCarrySecret(line: string): boolean {
  const [first = "", second = ""] = line.trim().split(/\s+/).map(asciiUpper);
  if (["AUTH", "HELLO", "MIGRATE"].includes(first)) return true;
  if (first === "ACL" && second === "SETUSER") return true;
  return first === "CONFIG" && second === "SET";
}
