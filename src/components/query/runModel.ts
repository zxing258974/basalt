// 执行的纯逻辑:要执行哪几条语句、每条的结果怎么说、结果分成哪几个标签。不碰后端和 DOM。
import type { QueryScope, ResultSet, StatementOutcome } from "../../lib/queryTypes";
import { splitStatements, statementAtCursor, type SqlDialect, type SqlStatement } from "./splitStatements";

export type RunMode = "current" | "all";

/**
 * 要执行的语句,位置是整篇文本里的。「当前」:有选中文字就执行选中的部分(可以是多条),
 * 否则执行光标所在的那条;「全部」:整篇。
 */
export function statementsToRun(
  text: string,
  selection: { from: number; to: number },
  dialect: SqlDialect,
  mode: RunMode,
): SqlStatement[] {
  if (mode === "all") return splitStatements(text, dialect);
  if (selection.from !== selection.to) {
    const offset = Math.min(selection.from, selection.to);
    const picked = text.slice(offset, Math.max(selection.from, selection.to));
    return splitStatements(picked, dialect).map((s) => ({
      ...s,
      from: s.from + offset,
      to: s.to + offset,
      end: s.end + offset,
    }));
  }
  const current = statementAtCursor(splitStatements(text, dialect), text, selection.to);
  return current ? [current] : [];
}

export interface RunEntry {
  sql: string;
  /** null:还在执行。 */
  outcome: StatementOutcome | null;
  /** 没能执行(连接关了之类)的原因。 */
  failure: string | null;
}

export interface RunState {
  running: boolean;
  /** 这次要执行的语句总数。 */
  total: number;
  entries: RunEntry[];
  /** 用户点了取消。 */
  cancelRequested: boolean;
}

export const IDLE: RunState = { running: false, total: 0, entries: [], cancelRequested: false };

export type EntryStatus = "running" | "ok" | "error" | "cancelled";

export function entryStatus(entry: RunEntry): EntryStatus {
  if (entry.failure !== null) return "error";
  if (entry.outcome === null) return "running";
  return entry.outcome.status;
}

export function formatMs(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(2)} s`;
  const minutes = Math.floor(ms / 60_000);
  return `${minutes} 分 ${Math.round((ms % 60_000) / 1000)} 秒`;
}

function rowsText(sets: readonly ResultSet[]): string {
  const rows = sets.reduce((sum, set) => sum + set.totalRows, 0);
  const cut = sets.some((set) => set.truncated) ? "（结果太大，后面的行没有保留）" : "";
  const many = sets.length > 1 ? `${sets.length} 个结果集，共 ` : "";
  return `${many}${rows} 行${cut}`;
}

/** 消息页里一条语句的结果说明。 */
export function entryDetail(entry: RunEntry): string {
  if (entry.failure !== null) return entry.failure;
  const outcome = entry.outcome;
  if (outcome === null) return "执行中…";
  if (outcome.status === "cancelled") return `已取消：${outcome.message ?? ""}`.replace(/：$/, "");
  if (outcome.status === "error") return outcome.message ?? "执行出错";
  if (outcome.resultSets.length > 0) return rowsText(outcome.resultSets);
  if (outcome.affectedRows !== null) return `影响 ${outcome.affectedRows} 行`;
  return "执行成功";
}

/** 消息页顶上的一句总结。 */
export function runSummary(state: RunState): string {
  const { entries, total } = state;
  if (entries.length === 0) return state.running ? "正在执行…" : "";
  const elapsed = entries.reduce((sum, e) => sum + (e.outcome?.elapsedMs ?? 0), 0);
  const last = entries.at(-1)!;
  const status = entryStatus(last);
  const skipped = total - entries.length;
  const rest = skipped > 0 ? `，后面 ${skipped} 条没有执行` : "";
  if (status === "running") return `正在执行第 ${entries.length} 条，共 ${total} 条…`;
  if (status === "error") return `第 ${entries.length} 条出错${rest}。`;
  if (status === "cancelled" || (state.cancelRequested && skipped > 0)) {
    return `已取消：执行了 ${entries.length} 条${rest}。`;
  }
  return `${total} 条语句执行成功，用时 ${formatMs(elapsed)}。`;
}

export interface ResultTab {
  key: string;
  label: string;
  /** 第几条语句的结果(重新执行时用)。 */
  entry: number;
  /** 产生它的那条语句,标签的提示里显示。 */
  sql: string;
  /** 执行那条语句时所在的库 / schema:导出全部结果要重新执行时回到这里,不跟着标签页后来换的库走。 */
  scope: QueryScope;
  set: ResultSet;
}

/** 每个结果集一个标签,按执行顺序编号。 */
export function resultTabs(entries: readonly RunEntry[]): ResultTab[] {
  return entries
    .flatMap((entry, i) =>
      (entry.outcome?.resultSets ?? []).map((set, j) => ({
        key: `r${i}-${j}`,
        label: "",
        entry: i,
        sql: entry.sql,
        scope: entry.outcome!.scope,
        set,
      })),
    )
    .map((tab, n) => ({ ...tab, label: `结果 ${n + 1}` }));
}
