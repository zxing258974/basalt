// Redis 的数据结构,与 crates/basalt-core/src/redis_db 的序列化结构一一对应。
import type { TablePage } from "./types";

/** 一个库:键数和设了过期时间的键数。 */
export interface RedisDatabase {
  index: number;
  keys: number;
  expires: number;
}

export interface RedisKeyspace {
  /** 服务器上一共有几个库。 */
  count: number;
  /** 对象树里列出的库:不多于 16 个时全部列出,更多时只列有键的、0 号库和默认库。 */
  databases: RedisDatabase[];
  /** 连接配置里的库号。 */
  defaultDb: number;
}

/** 键的名字。`id` 回传给后端定位键(二进制的键名按十六进制),`name` 给人看。 */
export interface KeyName {
  id: string;
  name: string;
  /** 键名不是 UTF-8,`name` 是 0x 开头的十六进制。 */
  binary: boolean;
}

/** TYPE 的回复;模块的类型名(如 ReJSON-RL)也可能出现。 */
export type RedisKind = "string" | "hash" | "list" | "set" | "zset" | "stream" | (string & {});

export interface KeySummary {
  key: KeyName;
  kind: RedisKind;
  /** 剩余的毫秒数;不过期为 null。 */
  ttlMs: number | null;
}

export interface ScanRequest {
  /** SCAN 的 MATCH 模式。 */
  pattern: string;
  kind: RedisKind | null;
  /** 从头扫是 "0"。 */
  cursor: string;
  /** 凑够这么多个就先回来。 */
  count: number;
}

export interface ScanPage {
  keys: KeySummary[];
  /** "0" 表示扫完了。 */
  cursor: string;
}

export type KeyValue =
  | { kind: "text"; text: string; truncated: boolean }
  | { kind: "bytes"; hex: string; truncated: boolean }
  | { kind: "table"; page: TablePage; truncated: boolean }
  | { kind: "unsupported"; reason: string };

export interface KeyDetail {
  key: KeyName;
  kind: RedisKind;
  ttlMs: number | null;
  /** 字符串的字节数,集合类的项数。 */
  length: number | null;
  memory: number | null;
  encoding: string | null;
  value: KeyValue;
}

export interface NewKey {
  name: string;
  kind: RedisKind;
  ttlSeconds: number | null;
  /** hash、stream 的第一个字段。 */
  field: string | null;
  /** 字符串的值,集合类的第一个元素 / 成员,hash、stream 的第一个值。 */
  value: string;
  /** zset 的第一个分数。 */
  score: string | null;
}

/** 能新建、能在类型筛选里选的几种。 */
export const REDIS_KINDS = ["string", "hash", "list", "set", "zset", "stream"] as const;

/** 命令行里一条命令的回复,结构照 RESP 原样。整数按字符串传(64 位整数在 number 里会丢精度)。 */
export type Reply =
  | { kind: "nil" }
  | { kind: "status"; text: string }
  | { kind: "error"; text: string }
  | { kind: "int"; value: string }
  | { kind: "number"; text: string }
  | { kind: "bool"; value: boolean }
  /** 不是 UTF-8 时 `text` 是 `\xHH` 转义过的,`binary` 为真;`truncated` 时只带回了前面一段,原文 `bytes` 字节。 */
  | { kind: "bulk"; text: string; binary: boolean; bytes: number; truncated: boolean }
  /** `total` 比 `items` 多时后面的没带回来。 */
  | { kind: "array"; items: Reply[]; total: number }
  | { kind: "map"; entries: [Reply, Reply][]; total: number };

export type ConsoleOutcome =
  /** 执行了。`db` 是执行后这条连接所在的库,`transaction` 表示在 MULTI 里。 */
  | { outcome: "ran"; reply: Reply; db: number; transaction: boolean; elapsedMs: number }
  /** 危险的命令,没有执行;确认之后带上 `confirmed` 再发一次。 */
  | { outcome: "confirm"; reason: string };
