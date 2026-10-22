import { describe, expect, it } from "vitest";
import type { ResultSet, StatementOutcome } from "../../lib/queryTypes";
import {
  entryDetail,
  entryStatus,
  formatMs,
  IDLE,
  resultTabs,
  runSummary,
  statementsToRun,
  type RunEntry,
} from "./runModel";

const scope = { database: "shop", schema: null };

function outcome(partial: Partial<StatementOutcome> = {}): StatementOutcome {
  return {
    status: "ok",
    message: null,
    resultSets: [],
    affectedRows: null,
    elapsedMs: 5,
    scope,
    notice: null,
    messages: [],
    errorPosition: null,
    ...partial,
  };
}

/** 后端存着 `rows` 行的结果集;随结果带回前端的最多一行,够看出行数按后端的总数算。 */
const set = (rows: number, truncated = false): ResultSet => ({
  id: rows,
  columns: [{ name: "n", dataType: "int", source: null }],
  rows: Array.from({ length: Math.min(rows, 1) }, (_, i) => [{ kind: "int", value: String(i) }]),
  totalRows: rows,
  truncated,
  readOnly: null,
});

const entry = (sql: string, result: StatementOutcome | null, failure: string | null = null): RunEntry => ({
  sql,
  outcome: result,
  failure,
});

describe("statementsToRun", () => {
  const text = "SELECT 1;\nSELECT 2;\nSELECT 3;";

  it("runs the statement under the cursor", () => {
    const [only] = statementsToRun(text, { from: 13, to: 13 }, "mysql", "current");
    expect(only?.text).toBe("SELECT 2");
  });

  it("runs everything in the selection, with positions in the whole text", () => {
    const from = text.indexOf("2");
    const picked = statementsToRun(text, { from, to: text.length }, "mysql", "current");
    expect(picked.map((s) => s.text)).toEqual(["2", "SELECT 3"]);
    expect(text.slice(picked[1]!.from, picked[1]!.to)).toBe("SELECT 3");
  });

  it("runs all statements, or none for an empty editor", () => {
    expect(statementsToRun(text, { from: 0, to: 0 }, "mysql", "all")).toHaveLength(3);
    expect(statementsToRun("  -- 只有注释", { from: 0, to: 0 }, "postgres", "current")).toEqual([]);
  });
});

describe("entry text", () => {
  it("describes each kind of result", () => {
    expect(entryDetail(entry("x", null))).toBe("执行中…");
    expect(entryStatus(entry("x", null))).toBe("running");
    expect(entryDetail(entry("x", outcome({ resultSets: [set(3)] })))).toBe("3 行");
    expect(entryDetail(entry("x", outcome({ resultSets: [set(2), set(1)] })))).toBe("2 个结果集，共 3 行");
    expect(entryDetail(entry("x", outcome({ resultSets: [set(5000, true)] })))).toBe(
      "5000 行（结果太大，后面的行没有保留）",
    );
    expect(entryDetail(entry("x", outcome({ affectedRows: 0 })))).toBe("影响 0 行");
    expect(entryDetail(entry("x", outcome()))).toBe("执行成功");
    expect(entryDetail(entry("x", outcome({ status: "error", message: "1146 - nope" })))).toBe("1146 - nope");
    expect(entryDetail(entry("x", outcome({ status: "error" })))).toBe("执行出错");
    expect(entryDetail(entry("x", outcome({ status: "cancelled", message: "interrupted" })))).toBe(
      "已取消：interrupted",
    );
    expect(entryDetail(entry("x", outcome({ status: "cancelled" })))).toBe("已取消");
    expect(entryDetail(entry("x", null, "连接尚未打开"))).toBe("连接尚未打开");
    expect(entryStatus(entry("x", null, "连接尚未打开"))).toBe("error");
  });

  it("formats durations", () => {
    expect(formatMs(12)).toBe("12 ms");
    expect(formatMs(1234)).toBe("1.23 s");
    expect(formatMs(65_000)).toBe("1 分 5 秒");
  });
});

describe("runSummary", () => {
  it("sums up a whole run", () => {
    expect(runSummary(IDLE)).toBe("");
    expect(runSummary({ ...IDLE, running: true, total: 2 })).toBe("正在执行…");
    const ok = [entry("a", outcome({ elapsedMs: 10 })), entry("b", outcome({ elapsedMs: 15 }))];
    expect(runSummary({ ...IDLE, total: 2, entries: ok })).toBe("2 条语句执行成功，用时 25 ms。");
    expect(runSummary({ ...IDLE, running: true, total: 3, entries: [...ok, entry("c", null)] })).toBe(
      "正在执行第 3 条，共 3 条…",
    );
    const failed = [entry("a", outcome()), entry("b", outcome({ status: "error" }))];
    expect(runSummary({ ...IDLE, total: 4, entries: failed })).toBe("第 2 条出错，后面 2 条没有执行。");
    const cancelled = [entry("a", outcome({ status: "cancelled" }))];
    expect(runSummary({ ...IDLE, total: 2, entries: cancelled })).toBe("已取消：执行了 1 条，后面 1 条没有执行。");
    // 取消请求到达前这条已经跑完:照实说它成功了,后面的没执行。
    expect(runSummary({ ...IDLE, total: 2, entries: [entry("a", outcome())], cancelRequested: true })).toBe(
      "已取消：执行了 1 条，后面 1 条没有执行。",
    );
  });
});

describe("resultTabs", () => {
  it("numbers result sets across statements", () => {
    const entries = [
      entry("CALL p()", outcome({ resultSets: [set(1), set(2)] })),
      entry("UPDATE t", outcome({ affectedRows: 1 })),
      entry("SELECT 1", outcome({ resultSets: [set(1)] })),
      entry("SELECT 2", null),
    ];
    expect(resultTabs(entries).map((t) => [t.key, t.label, t.sql, t.entry])).toEqual([
      ["r0-0", "结果 1", "CALL p()", 0],
      ["r0-1", "结果 2", "CALL p()", 0],
      ["r2-0", "结果 3", "SELECT 1", 2],
    ]);
  });

  it("remembers where each result was produced", () => {
    const entries = [
      entry("SELECT 1", outcome({ resultSets: [set(1)], scope: { database: "shop", schema: null } })),
      entry("USE shop_archive", outcome({ scope: { database: "shop_archive", schema: null } })),
      entry("SELECT 2", outcome({ resultSets: [set(1)], scope: { database: "shop_archive", schema: null } })),
    ];
    expect(resultTabs(entries).map((t) => t.scope.database)).toEqual(["shop", "shop_archive"]);
  });
});
