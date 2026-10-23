// 服务器监控的换算:前后两份快照相减得每秒多少,攒成一段走势;格式化;会话列表的筛选和排序。
import { formatBytes, formatCount } from "../../lib/format";
import type { ProcessActivity, ServerCounters, ServerProcess, ServerSnapshot } from "../../lib/monitorTypes";

/** 走势最多留多少个点(2 秒一次就是最近两分钟)。 */
export const HISTORY_LIMIT = 60;

/** 两份快照之间的每秒速率;某项读不到或计数器往回走了(服务器重启过)为 null。 */
export interface RatePoint {
  at: number;
  queries: number | null;
  commits: number | null;
  rollbacks: number | null;
  received: number | null;
  sent: number | null;
  /** 这段时间里读数据页命中缓存的比例,0 到 1。 */
  hitRate: number | null;
}

export interface MonitorHistory {
  /** 打开监控后的第一份,算「打开以来」的增量。 */
  first: ServerSnapshot | null;
  last: ServerSnapshot | null;
  points: RatePoint[];
}

export const EMPTY_HISTORY: MonitorHistory = { first: null, last: null, points: [] };

function perSecond(before: number | null, after: number | null, seconds: number): number | null {
  if (before === null || after === null || after < before) return null;
  return (after - before) / seconds;
}

/**
 * 两份快照之间服务器重启过:运行时长变短了;Redis 的 run_id 变了也算
 * (云上同一个地址主备切换到另一个节点,运行时长不一定变短,计数却换了一套)。
 */
function restarted(before: ServerSnapshot, after: ServerSnapshot): boolean {
  const a = before.counters.uptimeSeconds;
  const b = after.counters.uptimeSeconds;
  const runA = before.redis?.runId;
  const runB = after.redis?.runId;
  return (a !== null && b !== null && b < a) || (runA != null && runB != null && runA !== runB);
}

export function ratesBetween(before: ServerSnapshot, after: ServerSnapshot): RatePoint | null {
  const seconds = (after.takenAtMs - before.takenAtMs) / 1000;
  const a = before.counters;
  const b = after.counters;
  if (seconds <= 0 || restarted(before, after)) return null;
  const rate = (key: keyof ServerCounters) => perSecond(a[key], b[key], seconds);
  const reads = rate("cacheReads");
  const misses = rate("cacheMisses");
  return {
    at: after.takenAtMs,
    queries: rate("queries"),
    commits: rate("commits"),
    rollbacks: rate("rollbacks"),
    received: rate("bytesReceived"),
    sent: rate("bytesSent"),
    hitRate: reads !== null && misses !== null && reads > 0 ? Math.min(1, Math.max(0, 1 - misses / reads)) : null,
  };
}

/**
 * 接上新的一份快照。和上一份算不出速率时(本机时间往回拨了、服务器重启过)这一次不添点,
 * 但照样当作最新的一份,下一次接着算。服务器重启过时「打开以来」也从这一份重新算。
 */
export function recordSnapshot(
  history: MonitorHistory,
  snapshot: ServerSnapshot,
  limit = HISTORY_LIMIT,
): MonitorHistory {
  const { last } = history;
  const point = last ? ratesBetween(last, snapshot) : null;
  const restart = last !== null && restarted(last, snapshot);
  return {
    first: restart ? snapshot : (history.first ?? snapshot),
    last: snapshot,
    points: point ? [...history.points, point].slice(-limit) : history.points,
  };
}

/** 整个运行期间的缓存命中率:还没有两份快照可比时先用它。 */
export function lifetimeHitRate(counters: ServerCounters): number | null {
  const { cacheReads: reads, cacheMisses: misses } = counters;
  if (reads === null || misses === null || reads <= 0) return null;
  return Math.min(1, Math.max(0, 1 - misses / reads));
}

/** 打开监控以来涨了多少;计数器往回走了(服务器重启过)为 null。 */
export function growthSince(first: ServerSnapshot | null, last: ServerSnapshot, key: keyof ServerCounters) {
  const before = first?.counters[key];
  const after = last.counters[key];
  if (before === null || before === undefined || after === null || after < before) return null;
  return after - before;
}

// ---------- 格式化 ----------

const UNKNOWN = "—";
const decimal = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

/** 每秒多少次:10 以下留一位小数。 */
export function formatPerSecond(value: number | null): string {
  if (value === null) return UNKNOWN;
  return value < 10 ? decimal.format(value) : formatCount(Math.round(value));
}

export function formatBytesPerSecond(value: number | null): string {
  return value === null ? UNKNOWN : `${formatBytes(Math.round(value))}/s`;
}

/** 命中率常年在 99% 以上,留两位小数才看得出变化。 */
export function formatPercent(ratio: number | null): string {
  return ratio === null ? UNKNOWN : `${(ratio * 100).toFixed(2)}%`;
}

const UNITS: [seconds: number, label: string][] = [
  [86_400, "天"],
  [3_600, "小时"],
  [60, "分"],
  [1, "秒"],
];

/** 最大的两个单位:「3 天 4 小时」「5 分 12 秒」「8 秒」。 */
export function formatDuration(seconds: number | null): string {
  if (seconds === null || seconds < 0) return UNKNOWN;
  const total = Math.floor(seconds);
  const first = UNITS.findIndex(([size]) => total >= size);
  if (first < 0) return "0 秒";
  const parts: string[] = [];
  let rest = total;
  for (const [size, label] of UNITS.slice(first, first + 2)) {
    const amount = Math.floor(rest / size);
    rest -= amount * size;
    if (amount > 0 || parts.length === 0) parts.push(`${amount} ${label}`);
  }
  return parts.join(" ");
}

const pad = (n: number) => String(n).padStart(2, "0");

export function formatClock(ms: number): string {
  const at = new Date(ms);
  return `${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`;
}

// ---------- 会话列表 ----------

export interface ProcessFilter {
  hideIdle: boolean;
  search: string;
}

/** 排在前面的先看:在执行的、开着事务的(可能占着锁)、看不到的,最后是空闲的和后台线程。 */
const ORDER: Record<ProcessActivity, number> = {
  running: 0,
  inTransaction: 1,
  unknown: 2,
  idle: 3,
  background: 4,
};

/** 「隐藏空闲」藏掉的:空闲的连接和服务器自己的后台线程。 */
const QUIET: ReadonlySet<ProcessActivity> = new Set(["idle", "background"]);

/** 同一类里在当前状态待得久的排前面;同样的按编号。 */
function byActivity(a: ServerProcess, b: ServerProcess): number {
  const order = ORDER[a.activity] - ORDER[b.activity];
  if (order !== 0) return order;
  const seconds = (b.seconds ?? -1) - (a.seconds ?? -1);
  return seconds !== 0 ? seconds : a.id - b.id;
}

export function visibleProcesses(processes: readonly ServerProcess[], filter: ProcessFilter): ServerProcess[] {
  const needle = filter.search.trim().toLowerCase();
  const matches = (p: ServerProcess) =>
    !needle ||
    [String(p.id), p.user, p.host, p.database, p.state, p.detail, p.query].some((field) =>
      field?.toLowerCase().includes(needle),
    );
  return processes.filter((p) => !(filter.hideIdle && QUIET.has(p.activity)) && matches(p)).sort(byActivity);
}

export function runningCount(processes: readonly ServerProcess[]): number {
  return processes.filter((p) => p.activity === "running").length;
}
