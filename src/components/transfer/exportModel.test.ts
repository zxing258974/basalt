import { describe, expect, it } from "vitest";
import type { ResultSet, ResultSort } from "../../lib/queryTypes";
import {
  DEFAULT_CSV,
  buildFormat,
  buildSource,
  defaultRange,
  extensionOf,
  fileFilter,
  formatBytes,
  formatCount,
  formatElapsed,
  guessTableName,
  phaseLabel,
  querySummary,
  safeFileName,
  type ExportTarget,
} from "./exportModel";

const table = { database: "shop", schema: null, table: "customers" };

function tableTarget(filter: string | null): ExportTarget {
  return {
    connectionId: "c",
    driver: "mysql",
    title: "shop.customers",
    baseName: "customers",
    scope: { kind: "table", table, filter, sort: { column: "id", descending: true } },
  };
}

function queryTarget(set: ResultSet, sort: ResultSort | null = null): ExportTarget {
  return {
    connectionId: "c",
    driver: "postgres",
    title: "查询 1 的结果 1",
    baseName: "查询 1",
    scope: {
      kind: "query",
      editorId: "tab",
      scope: { database: "folio", schema: "public" },
      sql: "SELECT 1",
      set,
      sort,
    },
  };
}

describe("exportModel", () => {
  it("builds the format the backend expects", () => {
    const choices = { csv: DEFAULT_CSV, jsonLines: true, sqlTable: " users ", rowsPerStatement: 0 };
    expect(buildFormat({ ...choices, kind: "csv" })).toEqual({ kind: "csv", ...DEFAULT_CSV });
    expect(buildFormat({ ...choices, kind: "xlsx" })).toEqual({ kind: "xlsx" });
    expect(buildFormat({ ...choices, kind: "json" })).toEqual({ kind: "json", lines: true });
    expect(buildFormat({ ...choices, kind: "sql" })).toEqual({ kind: "sql", table: "users", rowsPerStatement: 1 });
  });

  it("picks extensions and dialog filters per format", () => {
    expect(extensionOf({ kind: "csv", ...DEFAULT_CSV })).toBe("csv");
    expect(extensionOf({ kind: "csv", ...DEFAULT_CSV, delimiter: "\t" })).toBe("tsv");
    expect(extensionOf({ kind: "json", lines: true })).toBe("jsonl");
    expect(extensionOf({ kind: "json", lines: false })).toBe("json");
    expect(fileFilter({ kind: "xlsx" })).toEqual({ name: "Excel 工作簿", extensions: ["xlsx"] });
    expect(fileFilter({ kind: "sql", table: "t", rowsPerStatement: 1 }).extensions).toEqual(["sql"]);
    expect(safeFileName("a/b:c*?")).toBe("a_b_c__");
    expect(safeFileName("  ")).toBe("export");
  });

  it("table exports carry the filter only when chosen", () => {
    expect(defaultRange(tableTarget("id > 5"))).toBe("filtered");
    expect(defaultRange(tableTarget(null))).toBe("all");
    expect(buildSource(tableTarget("id > 5"), "filtered")).toEqual({
      kind: "table",
      table,
      filter: "id > 5",
      sort: { column: "id", descending: true },
    });
    expect(buildSource(tableTarget("id > 5"), "all")).toEqual({ kind: "table", table, filter: null, sort: null });
  });

  it("query results rerun only when the backend couldn't keep them whole", () => {
    const set: ResultSet = {
      id: 7,
      columns: [{ name: "n", dataType: "int4", source: null }],
      rows: [[{ kind: "int", value: "1" }]],
      totalRows: 2500,
      truncated: false,
      readOnly: null,
    };
    expect(buildSource(queryTarget(set), "all")).toEqual({
      kind: "result",
      editorId: "tab",
      resultId: 7,
      columns: set.columns,
      sort: null,
    });
    expect(querySummary(set, null)).toBe("导出执行时读到的全部 2,500 行。");
    // 表格上点了排序:照它的顺序导出,说明里写明。
    const sort = { column: 0, descending: true };
    expect(buildSource(queryTarget(set, sort), "all")).toHaveProperty("sort", sort);
    expect(querySummary(set, sort)).toBe(`导出执行时读到的全部 2,500 行，按「${set.columns[0]!.name}」降序。`);
    const cut = { ...set, truncated: true };
    expect(buildSource(queryTarget(cut), "all")).toEqual({
      kind: "query",
      editorId: "tab",
      scope: { database: "folio", schema: "public" },
      sql: "SELECT 1",
    });
    expect(querySummary(cut, null)).toContain("重新执行");
    expect(querySummary(cut, sort)).toContain("表格上点的排序不算");
    expect(defaultRange(queryTarget(set))).toBe("all");
  });

  it("guesses a table name for INSERT statements", () => {
    expect(guessTableName("SELECT * FROM shop.`orders` WHERE id > 1")).toBe("orders");
    expect(guessTableName('select * from "Users"')).toBe("Users");
    expect(guessTableName("SELECT * FROM a JOIN b ON a.id = b.id")).toBe("query_result");
    expect(guessTableName("SELECT 1")).toBe("query_result");
  });

  it("formats counts, sizes, elapsed time and phases", () => {
    expect(formatCount(1234567)).toBe("1,234,567");
    expect(formatBytes(812)).toBe("812 B");
    expect(formatBytes(3 * 1024 + 300)).toBe("3.3 KB");
    expect(formatBytes(150 * 1024 * 1024)).toBe("150 MB");
    expect(formatBytes(2.5 * 1024 ** 3)).toBe("2.5 GB");
    expect(formatElapsed(75_400)).toBe("1:15");
    expect(phaseLabel("exporting")).toBe("正在导出");
    expect(phaseLabel("checking")).toBe("正在检查文件");
    expect(phaseLabel("importing")).toBe("正在导入");
    expect(phaseLabel("finishing")).toBe("正在收尾");
    expect(phaseLabel(null)).toBe("正在准备");
  });
});
