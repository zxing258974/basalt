// Redis 键浏览的纯逻辑:筛选框里的字怎么变成 SCAN 的模式、按冒号分组成树、剩余时间和大小怎么写。
import type { KeySummary, RedisKind } from "../../lib/redisTypes";

/** 键名里分层用的分隔符,和常见的 Redis 客户端一样。 */
export const SEPARATOR = ":";

/** 写了这些就当用户自己写的模式;`\` 和单独的 `]` 只是普通的字。 */
const GLOB = /[*?[]/;

/**
 * 筛选框里的字 → SCAN 的 MATCH 模式:空的是全部;写了 * ? [ 的照用户写的;
 * 普通的字前后加 *,按「名字里含有」找(里面的反斜杠转义成字面的反斜杠)。
 */
export function scanPattern(filter: string): string {
  const text = filter.trim();
  if (!text) return "*";
  return GLOB.test(text) ? text : `*${text.replaceAll("\\", "\\\\")}*`;
}

export interface FolderRow {
  kind: "folder";
  /** 完整前缀,如 `user:1001`,也是展开状态记的名字。 */
  path: string;
  label: string;
  depth: number;
  /** 下面一共有几个键(包括更深的)。 */
  count: number;
  expanded: boolean;
}

export interface KeyRow {
  kind: "key";
  key: KeySummary;
  /** 去掉所在分组的前缀之后剩下的部分。 */
  label: string;
  depth: number;
}

export type BrowserRow = FolderRow | KeyRow;

interface Node {
  folders: Map<string, Node>;
  keys: { label: string; key: KeySummary }[];
  count: number;
}

const node = (): Node => ({ folders: new Map(), keys: [], count: 0 });

const collator = new Intl.Collator("zh-CN", { numeric: true });
const byName = (a: string, b: string) => collator.compare(a, b);

/**
 * 列表要显示的行。平铺时一键一行,按名字排(数字按大小:user:2 在 user:10 前面);
 * 分组时按冒号拆成文件夹,文件夹在前、键在后,只展开 `expanded` 里的("all" 全部展开:筛选时用)。
 * 二进制的键名不拆。
 */
export function browserRows(
  keys: readonly KeySummary[],
  grouped: boolean,
  expanded: ReadonlySet<string> | "all",
): BrowserRow[] {
  if (!grouped) {
    return [...keys]
      .sort((a, b) => byName(a.key.name, b.key.name))
      .map((key): KeyRow => ({ kind: "key", key, label: key.key.name, depth: 0 }));
  }
  const root = node();
  for (const key of keys) {
    const parts = key.key.binary ? [key.key.name] : key.key.name.split(SEPARATOR);
    let at = root;
    at.count += 1;
    for (const part of parts.slice(0, -1)) {
      const next = at.folders.get(part) ?? node();
      at.folders.set(part, next);
      next.count += 1;
      at = next;
    }
    at.keys.push({ label: parts.at(-1) ?? "", key });
  }
  const rows: BrowserRow[] = [];
  const walk = (at: Node, prefix: string, depth: number) => {
    for (const [label, child] of [...at.folders].sort((a, b) => byName(a[0], b[0]))) {
      const path = prefix ? `${prefix}${SEPARATOR}${label}` : label;
      const open = expanded === "all" || expanded.has(path);
      rows.push({ kind: "folder", path, label, depth, count: child.count, expanded: open });
      if (open) walk(child, path, depth + 1);
    }
    for (const { label, key } of [...at.keys].sort((a, b) => byName(a.label, b.label))) {
      rows.push({ kind: "key", key, label, depth });
    }
  };
  walk(root, "", 0);
  return rows;
}

/** 扫描分批回来,同一个键可能出现两次(SCAN 的保证只是「不漏」)。后来的为准。 */
export function mergeKeys(pages: readonly (readonly KeySummary[])[]): KeySummary[] {
  const byId = new Map<string, KeySummary>();
  for (const page of pages) for (const key of page) byId.set(key.key.id, key);
  return [...byId.values()];
}

/** 剩余时间写成「还剩 29 分 12 秒」里的那段;不过期是「不过期」。 */
export function formatTtl(ms: number | null): string {
  if (ms === null) return "不过期";
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return seconds % 60 ? `${minutes} 分 ${seconds % 60} 秒` : `${minutes} 分`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return minutes % 60 ? `${hours} 小时 ${minutes % 60} 分` : `${hours} 小时`;
  const days = Math.floor(hours / 24);
  return hours % 24 ? `${days} 天 ${hours % 24} 小时` : `${days} 天`;
}

/** 长度的说法:字符串是字节,其余是项。 */
export function formatLength(kind: RedisKind, length: number | null): string | null {
  if (length === null) return null;
  const n = length.toLocaleString();
  switch (kind) {
    case "string":
      return `${n} 字节`;
    case "hash":
      return `${n} 个字段`;
    case "stream":
      return `${n} 条消息`;
    case "list":
      return `${n} 个元素`;
    default:
      return `${n} 个成员`;
  }
}

/** 类型的说明(徽标上写英文类型名,鼠标停上去看这句)。 */
export const KIND_NAMES: Record<string, string> = {
  string: "字符串",
  hash: "哈希",
  list: "列表",
  set: "集合",
  zset: "有序集合",
  stream: "消息流",
};

/** 把「秒数」输入框里的字解析成秒;空的是不过期。 */
/** 过期时间最长 100 年(和后端一样);再长 Redis 会因为算出的时刻溢出而拒绝。 */
export const MAX_TTL_SECONDS = 100 * 365 * 86_400;

export function parseTtl(text: string): { ok: true; seconds: number | null } | { ok: false; message: string } {
  const value = text.trim();
  if (!value) return { ok: true, seconds: null };
  if (!/^\d+$/.test(value) || Number(value) < 1) return { ok: false, message: "过期时间要是至少 1 的整数（秒）" };
  if (Number(value) > MAX_TTL_SECONDS) return { ok: false, message: "过期时间最长 100 年；想一直留着就留空" };
  return { ok: true, seconds: Number(value) };
}
