// mock 后端的 Redis 命令行:每个标签页自己的库号和事务,常用的几十条命令照 Redis 的回复写,
// 危险的先要确认、订阅类拒绝,BLPOP 一直等到被停下。规则照着 crates/basalt-core/src/redis_db/console.rs。
import type { CommandError } from "../types";
import type { ConsoleOutcome, Reply } from "../redisTypes";
import { commandVerb, dangerReason, refusedReason, splitArgs } from "../redisArgs";
import type { MockRedis, MockRedisKey, MockRedisValue } from "./redisMock";

function fail(code: string, message: string): never {
  const error: CommandError = { code, message, fields: [] };
  throw error;
}

const ok: Reply = { kind: "status", text: "OK" };
const nil: Reply = { kind: "nil" };
const int = (n: number): Reply => ({ kind: "int", value: String(n) });
const bulk = (text: string): Reply => ({
  kind: "bulk",
  text,
  binary: false,
  bytes: new TextEncoder().encode(text).length,
  truncated: false,
});
const array = (items: Reply[]): Reply => ({ kind: "array", items, total: items.length });
const error = (text: string): Reply => ({ kind: "error", text });
const WRONGTYPE = error("WRONGTYPE Operation against a key holding the wrong kind of value");
const syntax = error("ERR syntax error");
const notInteger = error("ERR value is not an integer or out of range");

/** Redis 写分数的样子。 */
const scoreText = (s: number) => (s === Infinity ? "inf" : s === -Infinity ? "-inf" : String(s));

interface ConsoleState {
  db: number;
  /** MULTI 之后排队的命令;不在事务里是 null。 */
  queue: string[][] | null;
  /** 正在等的 BLPOP:停下时调它。 */
  stop: (() => void) | null;
}

/** 命令要几个参数(连命令名);-n 是至少 n 个。 */
const ARITY: Record<string, number> = {
  PING: -1,
  ECHO: 2,
  GET: 2,
  SET: -3,
  DEL: -2,
  EXISTS: -2,
  INCR: 2,
  TYPE: 2,
  TTL: 2,
  EXPIRE: 3,
  KEYS: 2,
  DBSIZE: 1,
  SELECT: 2,
  HGET: 3,
  HGETALL: 2,
  HSET: -4,
  LRANGE: 4,
  LPUSH: -3,
  RPUSH: -3,
  LLEN: 2,
  SMEMBERS: 2,
  SADD: -3,
  ZRANGE: -4,
  FLUSHDB: -1,
  INFO: -1,
  BLPOP: -3,
  AUTH: -2,
  MULTI: 1,
  EXEC: 1,
  DISCARD: 1,
};

export function createMockRedisConsole(redis: MockRedis) {
  const consoles = new Map<string, ConsoleState>();

  const stateOf = (id: string, db: number) => {
    const found = consoles.get(id);
    if (found) return found;
    const created: ConsoleState = { db, queue: null, stop: null };
    consoles.set(id, created);
    return created;
  };

  /** 执行一条(已经拆好、已经核对过参数个数的)命令。 */
  async function execute(state: ConsoleState, args: string[]): Promise<Reply> {
    const name = args[0]!.toUpperCase();
    const keys = redis.db(state.db);
    const get = (key: string): MockRedisKey | undefined => keys.get(key);
    const typed = <T extends MockRedisValue["type"]>(key: string, type: T) => {
      const entry = get(key);
      if (!entry) return null;
      if (entry.value.type !== type) return WRONGTYPE;
      return entry.value as Extract<MockRedisValue, { type: T }>;
    };
    const put = (key: string, value: MockRedisValue) =>
      keys.set(key, { value, expiresAt: get(key)?.expiresAt ?? null });
    const a = args.slice(1);
    switch (name) {
      case "PING":
        return a[0] === undefined ? { kind: "status", text: "PONG" } : bulk(a[0]);
      case "ECHO":
        return bulk(a[0]!);
      case "GET": {
        const value = typed(a[0]!, "string");
        return value === null ? nil : "kind" in value ? value : bulk(value.value);
      }
      case "SET":
        if (a.length > 2) return syntax;
        keys.set(a[0]!, { value: { type: "string", value: a[1]! }, expiresAt: null });
        return ok;
      case "DEL":
        return int(a.filter((k) => keys.delete(k)).length);
      case "EXISTS":
        return int(a.filter((k) => keys.has(k)).length);
      case "INCR": {
        const value = typed(a[0]!, "string");
        if (value !== null && "kind" in value) return value;
        const now = value === null ? 0 : Number(value.value);
        if (!Number.isSafeInteger(now) || (value !== null && !/^-?\d+$/.test(value.value))) return notInteger;
        put(a[0]!, { type: "string", value: String(now + 1) });
        return int(now + 1);
      }
      case "TYPE":
        return { kind: "status", text: get(a[0]!)?.value.type ?? "none" };
      case "TTL": {
        const entry = get(a[0]!);
        if (!entry) return int(-2);
        return int(entry.expiresAt === null ? -1 : Math.ceil((entry.expiresAt - Date.now()) / 1000));
      }
      case "EXPIRE": {
        const entry = get(a[0]!);
        if (!/^-?\d+$/.test(a[1]!)) return notInteger;
        if (!entry) return int(0);
        entry.expiresAt = Date.now() + Number(a[1]) * 1000;
        return int(1);
      }
      case "KEYS": {
        const pattern = new RegExp(
          `^${a[0]!
            .replace(/[.+^${}()|\\]/g, "\\$&")
            .replace(/\*/g, ".*")
            .replace(/\?/g, ".")}$`,
          "s",
        );
        return array(
          [...keys.keys()]
            .filter((k) => pattern.test(k))
            .sort()
            .map(bulk),
        );
      }
      case "DBSIZE":
        return int(keys.size);
      case "SELECT": {
        const db = Number(a[0]);
        if (!/^\d+$/.test(a[0]!)) return notInteger;
        if (db >= 16) return error("ERR DB index is out of range");
        state.db = db;
        return ok;
      }
      case "HGET": {
        const value = typed(a[0]!, "hash");
        if (value === null) return nil;
        if ("kind" in value) return value;
        const found = value.value.find(([f]) => f === a[1]);
        return found ? bulk(found[1]) : nil;
      }
      case "HGETALL": {
        const value = typed(a[0]!, "hash");
        if (value === null) return array([]);
        if ("kind" in value) return value;
        return array(value.value.flatMap(([f, v]) => [bulk(f), bulk(v)]));
      }
      case "HSET": {
        if (a.length % 2 === 0) return error("ERR wrong number of arguments for 'hset' command");
        const value = typed(a[0]!, "hash");
        if (value !== null && "kind" in value) return value;
        const pairs = value?.value ?? [];
        let added = 0;
        for (let i = 1; i < a.length; i += 2) {
          const at = pairs.findIndex(([f]) => f === a[i]);
          if (at >= 0) pairs[at] = [a[i]!, a[i + 1]!];
          else {
            pairs.push([a[i]!, a[i + 1]!]);
            added += 1;
          }
        }
        put(a[0]!, { type: "hash", value: pairs });
        return int(added);
      }
      case "LRANGE": {
        const value = typed(a[0]!, "list");
        if (value === null) return array([]);
        if ("kind" in value) return value;
        const n = value.value.length;
        const at = (s: string) => (Number(s) < 0 ? n + Number(s) : Number(s));
        return array(value.value.slice(Math.max(at(a[1]!), 0), at(a[2]!) + 1).map(bulk));
      }
      case "LPUSH":
      case "RPUSH": {
        const value = typed(a[0]!, "list");
        if (value !== null && "kind" in value) return value;
        const items = value?.value ?? [];
        const added = a.slice(1);
        const next = name === "LPUSH" ? [...added.reverse(), ...items] : [...items, ...added];
        put(a[0]!, { type: "list", value: next });
        return int(next.length);
      }
      case "LLEN": {
        const value = typed(a[0]!, "list");
        return value === null ? int(0) : "kind" in value ? value : int(value.value.length);
      }
      case "SMEMBERS": {
        const value = typed(a[0]!, "set");
        return value === null ? array([]) : "kind" in value ? value : array(value.value.map(bulk));
      }
      case "SADD": {
        const value = typed(a[0]!, "set");
        if (value !== null && "kind" in value) return value;
        const members = value?.value ?? [];
        const added = [...new Set(a.slice(1))].filter((m) => !members.includes(m));
        put(a[0]!, { type: "set", value: [...members, ...added] });
        return int(added.length);
      }
      case "ZRANGE": {
        const value = typed(a[0]!, "zset");
        if (value === null) return array([]);
        if ("kind" in value) return value;
        const sorted = [...value.value].sort((x, y) => x[1] - y[1] || (x[0] < y[0] ? -1 : 1));
        const n = sorted.length;
        const at = (s: string) => (Number(s) < 0 ? n + Number(s) : Number(s));
        const slice = sorted.slice(Math.max(at(a[1]!), 0), at(a[2]!) + 1);
        const scores = a[3]?.toUpperCase() === "WITHSCORES";
        return array(slice.flatMap(([m, s]) => (scores ? [bulk(m), bulk(scoreText(s))] : [bulk(m)])));
      }
      case "FLUSHDB":
        keys.clear();
        return ok;
      case "INFO":
        return bulk(
          "# Server\r\nredis_version:7.4.1\r\nredis_mode:standalone\r\ntcp_port:6379\r\nuptime_in_days:12\r\n\r\n# Keyspace\r\n" +
            `db${state.db}:keys=${keys.size},expires=${[...keys.values()].filter((k) => k.expiresAt !== null).length},avg_ttl=0\r\n`,
        );
      case "BLPOP":
        return blpop(state, a);
      case "AUTH":
        return error("WRONGPASS invalid username-password pair or user is disabled.");
      default:
        return error(`ERR unknown command '${args[0]}', with args beginning with: ${a.map((x) => `'${x}' `).join("")}`);
    }
  }

  /** BLPOP:有元素就弹出,没有就等到超时(0 是一直等)或被停下。 */
  function blpop(state: ConsoleState, a: string[]): Promise<Reply> {
    const keys = a.slice(0, -1);
    const pop = (): Reply | null => {
      for (const key of keys) {
        const entry = redis.db(state.db).get(key);
        if (entry?.value.type === "list" && entry.value.value.length > 0) {
          const item = entry.value.value.shift()!;
          if (entry.value.value.length === 0) redis.db(state.db).delete(key);
          return array([bulk(key), bulk(item)]);
        }
      }
      return null;
    };
    const now = pop();
    if (now) return Promise.resolve(now);
    const timeout = Number(a.at(-1));
    return new Promise((resolve, reject) => {
      const timer = timeout > 0 ? setTimeout(() => finish(() => resolve(nil)), timeout * 1000) : undefined;
      const finish = (then: () => void) => {
        clearTimeout(timer);
        state.stop = null;
        then();
      };
      const message = "命令行的连接断开了，下一条命令会重新连接（事务和 WATCH 已经失效）";
      state.stop = () => finish(() => reject(Object.assign(new Error(message), { code: "network", fields: [] })));
    });
  }

  return {
    async run(id: string, db: number, line: string, confirmed: boolean): Promise<ConsoleOutcome> {
      const args = splitArgs(line);
      if ("error" in args) fail("invalid_edit", args.error);
      if (args.length === 0) fail("invalid_edit", "请输入命令");
      const verb = commandVerb(args);
      const refused = refusedReason(verb);
      if (refused) fail("unsupported", refused);
      const danger = dangerReason(verb);
      if (danger && !confirmed) return { outcome: "confirm", reason: danger };

      const state = stateOf(id, db);
      const started = performance.now();
      const name = args[0]!.toUpperCase();
      const arity = ARITY[name];
      let reply: Reply;
      if (arity !== undefined && (arity > 0 ? args.length !== arity : args.length < -arity)) {
        reply = error(`ERR wrong number of arguments for '${args[0]!.toLowerCase()}' command`);
      } else if (name === "MULTI") {
        reply = state.queue ? error("ERR MULTI calls can not be nested") : ok;
        state.queue ??= [];
      } else if (name === "EXEC" || name === "DISCARD") {
        const queue = state.queue;
        state.queue = null;
        if (!queue) reply = error(`ERR ${name} without MULTI`);
        else if (name === "DISCARD") reply = ok;
        else {
          const replies: Reply[] = [];
          for (const queued of queue) replies.push(await execute(state, queued));
          reply = array(replies);
        }
      } else if (state.queue) {
        state.queue.push(args);
        reply = { kind: "status", text: "QUEUED" };
      } else {
        reply = await execute(state, args);
      }
      return {
        outcome: "ran",
        reply,
        db: state.db,
        transaction: state.queue !== null,
        // 只有真的等过(BLPOP)才记耗时;别的在本进程里立刻算完,记 0,截图才稳定。
        elapsedMs: name === "BLPOP" ? Math.round(performance.now() - started) : 0,
      };
    },
    /** 停下:正在等的命令失败,事务丢掉,下一条命令「重新连接」到界面上的库。 */
    cancel(id: string) {
      const state = consoles.get(id);
      state?.stop?.();
      consoles.delete(id);
    },
    close(id: string) {
      this.cancel(id);
    },
    commandNames(): string[] {
      return Object.keys(ARITY).sort();
    },
  };
}
