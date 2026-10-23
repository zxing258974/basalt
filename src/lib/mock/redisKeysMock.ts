// mock 后端的 Redis 键操作:扫描、读值、核对后提交修改、新建 / 改名 / 过期 / 删除。
// 规则照着 crates/basalt-core/src/redis_db 的 keys.rs、edit.rs,命令的写法也一样。
import type { CellValue, ColumnInfo, CommandError, RowChange, TablePage, ValueType } from "../types";
import type { KeyDetail, KeyName, KeySummary, NewKey, RedisKind, ScanPage, ScanRequest } from "../redisTypes";
import type { MockRedis, MockRedisKey, MockRedisValue } from "./redisMock";

const MAX_ITEMS = 1000;
/** 和后端的 `MAX_TTL_SECONDS` 一样。 */
const MAX_TTL_SECONDS = 100 * 365 * 86_400;
const TOMBSTONE = "\0basalt:deleted";

function fail(code: string, message: string): never {
  const error: CommandError = { code, message, fields: [] };
  throw error;
}

const keyName = (name: string): KeyName => ({ id: `s:${name}`, name, binary: false });

function nameOf(id: string): string {
  if (!id.startsWith("s:")) fail("invalid_edit", `认不出键「${id}」`);
  return id.slice(2);
}

/** SCAN 的 MATCH:* 任意串,? 一个字,[…] 字符集,\ 转义。 */
function globRegex(pattern: string): RegExp {
  let out = "";
  for (let i = 0; i < pattern.length; i += 1) {
    const ch = pattern[i]!;
    if (ch === "*") out += ".*";
    else if (ch === "?") out += ".";
    else if (ch === "\\" && i + 1 < pattern.length) out += `\\${pattern[++i]}`;
    else if (ch === "[") {
      const end = pattern.indexOf("]", i + 1);
      if (end < 0) out += "\\[";
      else {
        const body = pattern.slice(i + 1, end).replace(/^\^/, "^");
        out += `[${body}]`;
        i = end;
      }
    } else out += ch.replace(/[.+^${}()|]/g, "\\$&");
  }
  return new RegExp(`^${out}$`, "s");
}

/** 显示出来看不见、或者会把后面的字倒过来显示的字:控制字符、BOM、双向文字的控制符。 */
const HIDDEN = /[\p{Cc}\ufeff\u200e\u200f\u202a-\u202e\u2066-\u2069]/u;

/** 照 redis-cli 的写法加引号;看不见的字照 UTF-8 字节写成 \\xHH。 */
export function quote(text: string): string {
  if (text && !/[\s"'\\]/u.test(text) && !HIDDEN.test(text)) return text;
  const escaped = [...text]
    .map((c) => {
      if (c === '"') return '\\"';
      if (c === "\\") return "\\\\";
      if (c === "\n") return "\\n";
      if (c === "\r") return "\\r";
      if (c === "\t") return "\\t";
      if (!HIDDEN.test(c)) return c;
      return [...new TextEncoder().encode(c)].map((b) => `\\x${b.toString(16).padStart(2, "0")}`).join("");
    })
    .join("");
  return `"${escaped}"`;
}

const command = (verb: string, key: string, ...args: string[]) => [verb, quote(key), ...args.map(quote)].join(" ");

function column(name: string, dataType: string, key: boolean, valueType: ValueType = { kind: "text" }): ColumnInfo {
  return { name, dataType, isPrimaryKey: key, nullable: false, hasDefault: false, isGenerated: false, valueType };
}

const text = (value: string): CellValue => ({ kind: "text", value });

function table(
  columns: ColumnInfo[],
  rows: CellValue[][],
  length: number,
  sql: string,
  readOnly: string | null = null,
) {
  const page: TablePage = { columns, rows, hasMore: rows.length < length, readOnlyReason: readOnly, sql };
  return { kind: "table" as const, page, truncated: rows.length < length };
}

function lengthOf(value: MockRedisValue): number {
  return value.type === "string" ? new TextEncoder().encode(value.value).length : value.value.length;
}

function detail(name: string, entry: MockRedisKey): KeyDetail {
  const { value } = entry;
  const base = {
    key: keyName(name),
    kind: value.type,
    ttlMs: entry.expiresAt === null ? null : Math.max(0, entry.expiresAt - Date.now()),
    length: lengthOf(value),
    memory: 56 + JSON.stringify(value.value).length * 2,
    encoding: value.type === "string" ? (/^\d+$/.test(value.value) ? "int" : "embstr") : "listpack",
  };
  const byText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
  switch (value.type) {
    case "string":
      return { ...base, value: { kind: "text", text: value.value, truncated: false } };
    case "hash": {
      const rows = [...value.value].sort((a, b) => byText(a[0], b[0])).slice(0, MAX_ITEMS);
      return {
        ...base,
        value: table(
          [column("field", "字段", true), column("value", "值", false)],
          rows.map(([f, v]) => [text(f), text(v)]),
          value.value.length,
          `HSCAN ${quote(name)} 0 COUNT 1000`,
        ),
      };
    }
    case "list":
      return {
        ...base,
        value: table(
          [
            {
              ...column("index", "下标", true, { kind: "integer", unsigned: true }),
              isGenerated: true,
              hasDefault: true,
            },
            column("value", "值", false),
          ],
          value.value.slice(0, MAX_ITEMS).map((item, i) => [{ kind: "int", value: String(i) }, text(item)]),
          value.value.length,
          `LRANGE ${quote(name)} 0 ${MAX_ITEMS - 1}`,
        ),
      };
    case "set":
      return {
        ...base,
        value: table(
          [column("member", "成员", true)],
          [...value.value].sort(byText).map((m) => [text(m)]),
          value.value.length,
          `SSCAN ${quote(name)} 0 COUNT 1000`,
        ),
      };
    case "zset": {
      const sorted = [...value.value].sort((a, b) => a[1] - b[1] || byText(a[0], b[0]));
      return {
        ...base,
        value: table(
          [column("member", "成员", true), column("score", "分数", false, { kind: "float" })],
          sorted.map(([m, s]) => [text(m), { kind: "decimal", value: scoreText(s) }]),
          value.value.length,
          `ZRANGE ${quote(name)} 0 ${MAX_ITEMS - 1} WITHSCORES`,
        ),
      };
    }
    case "stream":
      return {
        ...base,
        value: table(
          [column("id", "消息 ID", true), column("fields", "字段", false)],
          value.value.map((entry) => [
            text(entry.id),
            { kind: "json", value: JSON.stringify(Object.fromEntries(entry.fields)) },
          ]),
          value.value.length,
          `XRANGE ${quote(name)} - + COUNT ${MAX_ITEMS}`,
          "stream 的消息只能追加和删除，不能改；在命令行里用 XADD、XDEL",
        ),
      };
  }
}

const cellText = (cell: CellValue | undefined): string =>
  cell && "value" in cell && typeof cell.value === "string" ? cell.value : fail("invalid_edit", "缺少原值，没法核对");

function edited(values: Record<string, string | null>, column: string): string | undefined {
  if (!(column in values)) return undefined;
  const value = values[column];
  if (value === null || value === undefined) fail("invalid_edit", "Redis 里没有 NULL，可以填空字符串");
  return value;
}

function checkTtl(seconds: number): void {
  if (seconds < 1) fail("invalid_edit", "过期时间至少 1 秒");
  if (seconds > MAX_TTL_SECONDS) fail("invalid_edit", "过期时间最长 100 年；想一直留着就选不过期");
}

/** 核对过的分数 → 数;inf 写法 JS 的 Number 不认。 */
const scoreValue = (text: string): number =>
  /^[+-]?inf$/i.test(text) ? (text.startsWith("-") ? -Infinity : Infinity) : Number(text);

/** Redis 写分数的样子:无穷是 inf、-inf。 */
const scoreText = (s: number): string => (s === Infinity ? "inf" : s === -Infinity ? "-inf" : String(s));

/** 和后端一样:Redis 不认的(溢出成无穷、下溢成 0、十六进制)先拦下,无穷要明写 inf。 */
function score(raw: string): string {
  const trimmed = raw.trim();
  if (/^[+-]?inf$/i.test(trimmed)) return trimmed;
  const decimal = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/.exec(trimmed);
  const n = Number(trimmed);
  const underflow = n === 0 && /[1-9]/.test(decimal?.[1] ?? "");
  if (decimal && Number.isFinite(n) && !underflow) return trimmed;
  return fail("invalid_edit", `分数「${trimmed}」不是 Redis 能用的数`);
}

interface Plan {
  commands: string[];
  /** 核对:不过就整批不写。 */
  checks: ((value: MockRedisValue) => void)[];
  writes: ((value: MockRedisValue) => void)[];
}

function planChanges(name: string, kind: RedisKind, changes: RowChange[]): Plan {
  const plan: Plan = { commands: [], checks: [], writes: [] };
  const changed = (what: string) =>
    fail("row_mismatch", `${what}已经被别人改过或删掉了，这批修改没有提交。请刷新后再改`);
  const exists = (what: string) => fail("row_mismatch", `${what}已经有了，这批修改没有提交`);
  const written = new Set<string>();
  const once = (item: string, label: string) => {
    if (written.has(item)) fail("invalid_edit", `同一批修改里${label} ${quote(item)} 出现了两次`);
    written.add(item);
  };
  let tombstones = false;
  for (const change of changes) {
    const original = (column: string) => cellText(change.kind === "insert" ? undefined : change.key[column]);
    const values = change.kind === "delete" ? {} : change.values;
    switch (kind) {
      case "hash": {
        type Hash = Extract<MockRedisValue, { type: "hash" }>;
        const field = change.kind === "insert" ? null : original("field");
        const old = change.kind === "insert" ? null : original("value");
        const get = (v: MockRedisValue, f: string) => (v as Hash).value.find((p) => p[0] === f)?.[1];
        const set = (v: MockRedisValue, f: string, x: string) => {
          const hash = v as Hash;
          const at = hash.value.findIndex((p) => p[0] === f);
          if (at >= 0) hash.value[at] = [f, x];
          else hash.value.push([f, x]);
        };
        const del = (v: MockRedisValue, f: string) => {
          (v as Hash).value = (v as Hash).value.filter((p) => p[0] !== f);
        };
        if (field !== null && old !== null) {
          plan.checks.push((v) => {
            if (get(v, field) !== old) changed(`字段 ${quote(field)}`);
          });
        }
        if (change.kind === "update") {
          const newField = edited(values, "field") ?? field!;
          const newValue = edited(values, "value") ?? old!;
          once(newField, "字段");
          if (newField !== field) {
            plan.checks.push((v) => {
              if (get(v, newField) !== undefined) exists(`字段 ${quote(newField)}`);
            });
            plan.commands.push(command("HDEL", name, field!));
            plan.writes.push((v) => del(v, field!));
          }
          plan.commands.push(command("HSET", name, newField, newValue));
          plan.writes.push((v) => set(v, newField, newValue));
        } else if (change.kind === "delete") {
          plan.commands.push(command("HDEL", name, field!));
          plan.writes.push((v) => del(v, field!));
        } else {
          const newField = edited(values, "field") ?? fail("invalid_edit", "新增的行要填「字段」");
          const newValue = edited(values, "value") ?? "";
          once(newField, "字段");
          plan.checks.push((v) => {
            if (get(v, newField) !== undefined) exists(`字段 ${quote(newField)}`);
          });
          plan.commands.push(command("HSET", name, newField, newValue));
          plan.writes.push((v) => set(v, newField, newValue));
        }
        break;
      }
      case "list": {
        type List = Extract<MockRedisValue, { type: "list" }>;
        if (change.kind === "insert") {
          const item = edited(values, "value") ?? "";
          plan.commands.push(command("RPUSH", name, item));
          plan.writes.push((v) => void (v as List).value.push(item));
          break;
        }
        const index = Number(original("index"));
        const old = original("value");
        plan.checks.push((v) => {
          if ((v as List).value[index] !== old) changed(`下标 ${index} 的元素`);
        });
        if (change.kind === "update") {
          const item = edited(values, "value");
          if (item !== undefined) {
            plan.commands.push(command("LSET", name, String(index), item));
            plan.writes.push((v) => void ((v as List).value[index] = item));
          }
        } else {
          tombstones = true;
          plan.commands.push(command("LSET", name, String(index), TOMBSTONE));
          plan.writes.push((v) => void ((v as List).value[index] = TOMBSTONE));
        }
        break;
      }
      case "set": {
        type Members = Extract<MockRedisValue, { type: "set" }>;
        const has = (v: MockRedisValue, m: string) => (v as Members).value.includes(m);
        if (change.kind === "insert") {
          const member = edited(values, "member") ?? fail("invalid_edit", "新增的行要填「成员」");
          once(member, "成员");
          plan.checks.push((v) => {
            if (has(v, member)) exists(`成员 ${quote(member)}`);
          });
          plan.commands.push(command("SADD", name, member));
          plan.writes.push((v) => void (v as Members).value.push(member));
          break;
        }
        const member = original("member");
        const next = change.kind === "update" ? edited(values, "member") : undefined;
        if (change.kind === "update" && next === undefined) break;
        plan.checks.push((v) => {
          if (!has(v, member)) changed(`成员 ${quote(member)}`);
        });
        plan.commands.push(command("SREM", name, member));
        plan.writes.push((v) => void ((v as Members).value = (v as Members).value.filter((m) => m !== member)));
        if (next !== undefined) {
          once(next, "成员");
          plan.checks.push((v) => {
            if (has(v, next)) exists(`成员 ${quote(next)}`);
          });
          plan.commands.push(command("SADD", name, next));
          plan.writes.push((v) => void (v as Members).value.push(next));
        }
        break;
      }
      case "zset": {
        type Sorted = Extract<MockRedisValue, { type: "zset" }>;
        const scoreOf = (v: MockRedisValue, m: string) => (v as Sorted).value.find((p) => p[0] === m)?.[1];
        const add = (v: MockRedisValue, m: string, s: string) => {
          const sorted = v as Sorted;
          sorted.value = [...sorted.value.filter((p) => p[0] !== m), [m, scoreValue(s)]];
        };
        const remove = (v: MockRedisValue, m: string) => {
          (v as Sorted).value = (v as Sorted).value.filter((p) => p[0] !== m);
        };
        if (change.kind === "insert") {
          const member = edited(values, "member") ?? fail("invalid_edit", "新增的行要填「成员」");
          const s = score(edited(values, "score") ?? fail("invalid_edit", "新增的行要填「分数」"));
          once(member, "成员");
          plan.checks.push((v) => {
            if (scoreOf(v, member) !== undefined) exists(`成员 ${quote(member)}`);
          });
          plan.commands.push(command("ZADD", name, s, member));
          plan.writes.push((v) => add(v, member, s));
          break;
        }
        const member = original("member");
        const oldScore = original("score");
        plan.checks.push((v) => {
          const now = scoreOf(v, member);
          if (now === undefined || scoreText(now) !== oldScore) changed(`成员 ${quote(member)}`);
        });
        if (change.kind === "delete") {
          plan.commands.push(command("ZREM", name, member));
          plan.writes.push((v) => remove(v, member));
          break;
        }
        const nextMember = edited(values, "member") ?? member;
        const nextScore = edited(values, "score");
        const s = nextScore === undefined ? oldScore : score(nextScore);
        once(nextMember, "成员");
        if (nextMember !== member) {
          plan.checks.push((v) => {
            if (scoreOf(v, nextMember) !== undefined) exists(`成员 ${quote(nextMember)}`);
          });
          plan.commands.push(command("ZREM", name, member));
          plan.writes.push((v) => remove(v, member));
        }
        plan.commands.push(command("ZADD", name, s, nextMember));
        plan.writes.push((v) => add(v, nextMember, s));
        break;
      }
      case "stream":
        fail("read_only", "stream 的消息只能追加和删除，不能改；在命令行里用 XADD、XDEL");
        break;
      default:
        fail("read_only", `Basalt 改不了 ${kind} 类型的值`);
    }
  }
  if (tombstones) {
    plan.commands.push(command("LREM", name, "0", TOMBSTONE));
    plan.writes.push((v) => {
      const list = v as Extract<MockRedisValue, { type: "list" }>;
      list.value = list.value.filter((item) => item !== TOMBSTONE);
    });
  }
  return plan;
}

export function createMockRedisKeys(redis: MockRedis) {
  const entry = (db: number, name: string) =>
    redis.db(db).get(name) ?? fail("not_found", "这个键已经不存在了（可能过期了，或者被删了）");

  return {
    scan(db: number, request: ScanRequest): ScanPage {
      const pattern = globRegex(request.pattern || "*");
      const all = [...redis.db(db).entries()]
        .filter(([name, e]) => pattern.test(name) && (request.kind === null || e.value.type === request.kind))
        .sort((a, b) => (a[0] < b[0] ? -1 : 1));
      const start = Number(request.cursor) || 0;
      const end = start + Math.max(1, request.count);
      const keys: KeySummary[] = all.slice(start, end).map(([name, e]) => ({
        key: keyName(name),
        kind: e.value.type,
        ttlMs: e.expiresAt === null ? null : Math.max(0, e.expiresAt - Date.now()),
      }));
      return { keys, cursor: end >= all.length ? "0" : String(end) };
    },
    read(db: number, id: string): KeyDetail {
      const name = nameOf(id);
      return detail(name, entry(db, name));
    },
    preview(id: string, kind: RedisKind, changes: RowChange[]): string[] {
      return planChanges(nameOf(id), kind, changes).commands;
    },
    apply(db: number, id: string, kind: RedisKind, changes: RowChange[], expected: string[]) {
      const name = nameOf(id);
      const plan = planChanges(name, kind, changes);
      if (JSON.stringify(plan.commands) !== JSON.stringify(expected)) {
        fail("invalid_edit", "要执行的命令和预览时不一样了，请重新预览");
      }
      const target = redis.db(db).get(name);
      if (target?.value.type !== kind) {
        fail("row_mismatch", "这个键已经不存在了（过期或被删），或者变成了别的类型，这批修改没有提交。请刷新");
      }
      plan.checks.forEach((check) => check(target.value));
      plan.writes.forEach((write) => write(target.value));
      if (lengthOf(target.value) === 0) redis.db(db).delete(name);
      return { statements: plan.commands.length, affectedRows: changes.length };
    },
    setString(db: number, id: string, original: string, value: string) {
      const target = entry(db, nameOf(id));
      if (target.value.type !== "string" || target.value.value !== original) {
        fail("row_mismatch", "这个值已经被别人改过或删掉了，这批修改没有提交。请刷新后再改");
      }
      target.value = { type: "string", value };
    },
    create(db: number, key: NewKey): KeyName {
      if (!key.name) fail("invalid_edit", "请填写键名");
      if (redis.db(db).has(key.name)) fail("already_exists", "已经有同名的键了，没有新建");
      const field = () => key.field || fail("invalid_edit", "请填写第一个字段");
      const values: Record<string, () => MockRedisValue> = {
        string: () => ({ type: "string", value: key.value }),
        hash: () => ({ type: "hash", value: [[field(), key.value]] }),
        list: () => ({ type: "list", value: [key.value] }),
        set: () => ({ type: "set", value: [key.value] }),
        zset: () => ({ type: "zset", value: [[key.value, scoreValue(score(key.score ?? ""))]] }),
        stream: () => ({ type: "stream", value: [{ id: `${Date.now()}-0`, fields: [[field(), key.value]] }] }),
      };
      const value = values[key.kind]?.() ?? fail("invalid_edit", `不能新建 ${key.kind} 类型的键`);
      if (key.ttlSeconds !== null) checkTtl(key.ttlSeconds);
      redis.db(db).set(key.name, {
        value,
        expiresAt: key.ttlSeconds === null ? null : Date.now() + key.ttlSeconds * 1000,
      });
      return keyName(key.name);
    },
    rename(db: number, id: string, name: string): KeyName {
      const old = nameOf(id);
      if (!name) fail("invalid_edit", "请填写新的键名");
      const keys = redis.db(db);
      const target = entry(db, old);
      if (keys.has(name)) fail("already_exists", `已经有叫 ${quote(name)} 的键了，没有改名`);
      keys.delete(old);
      keys.set(name, target);
      return keyName(name);
    },
    remove(db: number, ids: string[]): number {
      const keys = redis.db(db);
      return ids.map(nameOf).filter((name) => keys.delete(name)).length;
    },
    setTtl(db: number, id: string, seconds: number | null) {
      if (seconds !== null) checkTtl(seconds);
      const target = redis.db(db).get(nameOf(id)) ?? fail("not_found", "这个键已经不存在了");
      target.expiresAt = seconds === null ? null : Date.now() + seconds * 1000;
    },
  };
}
