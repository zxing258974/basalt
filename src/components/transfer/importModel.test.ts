import { describe, expect, it } from "vitest";
import type { ImportReport, SourcePreview, UniqueKey } from "../../lib/transferTypes";
import type { ColumnInfo } from "../../lib/types";
import {
  affectedLabel,
  commitNote,
  confirmation,
  defaultKeys,
  describeFormat,
  describeKey,
  failuresFileName,
  isDestructive,
  keyProblem,
  mappedCount,
  mappedPreview,
  missingRequired,
  mysqlUpsertNote,
  needsSheetChoice,
  reportTitle,
  reportTone,
  setSource,
} from "./importModel";

function column(name: string, partial: Partial<ColumnInfo> = {}): ColumnInfo {
  return {
    name,
    dataType: "int",
    isPrimaryKey: false,
    nullable: true,
    hasDefault: false,
    isGenerated: false,
    valueType: { kind: "text" },
    ...partial,
  };
}

const preview: SourcePreview = {
  format: { kind: "csv", encoding: "utf8", delimiter: ",", quote: '"' },
  sheets: [],
  headers: ["名字", "编号"],
  rows: [
    ["张三", "1"],
    [null, "2"],
  ],
  rowNumbers: [2, 3],
};

function report(partial: Partial<ImportReport>): ImportReport {
  return { status: "completed", affected: 1, unmatched: 0, failed: 0, failures: [], summary: "", ...partial };
}

const keys: UniqueKey[] = [
  { name: null, columns: ["id"] },
  { name: "uk_code", columns: ["code", "region"] },
];
const mapping = (...columns: string[]) => columns.map((column, source) => ({ column, source }));

describe("importModel", () => {
  it("changes one mapping at a time without touching the rest", () => {
    const mapping = [
      { column: "id", source: 1 },
      { column: "name", source: null },
    ];
    const next = setSource(mapping, "name", 0);
    expect(next).toEqual([
      { column: "id", source: 1 },
      { column: "name", source: 0 },
    ]);
    expect(mapping[1]!.source).toBeNull();
    expect(mappedCount(next)).toBe(2);
    expect(mappedCount(setSource(next, "id", null))).toBe(1);
  });

  it("flags required columns that nobody fills", () => {
    const columns = [
      column("id", { nullable: false, hasDefault: true }),
      column("name", { nullable: false }),
      column("note"),
      column("label", { nullable: false, isGenerated: true }),
    ];
    expect(missingRequired(columns, [{ column: "id", source: null }]).map((c) => c.name)).toEqual(["name"]);
    expect(missingRequired(columns, [{ column: "name", source: 0 }])).toEqual([]);
  });

  it("previews only mapped columns in target order", () => {
    const shown = mappedPreview(preview, [
      { column: "id", source: 1 },
      { column: "note", source: null },
      { column: "name", source: 0 },
    ]);
    expect(shown.headers).toEqual(["id", "name"]);
    expect(shown.sources).toEqual(["编号", "名字"]);
    expect(shown.rows).toEqual([
      ["1", "张三"],
      ["2", null],
    ]);
    expect(mappedPreview(preview, [{ column: "x", source: 5 }]).sources).toEqual(["第 6 列"]);
  });

  it("describes formats", () => {
    expect(describeFormat(preview.format)).toBe("CSV · UTF-8 · 逗号分隔");
    expect(describeFormat({ kind: "csv", encoding: "gbk", delimiter: "\t", quote: '"' })).toBe(
      "CSV · GBK · 制表符分隔",
    );
    expect(describeFormat({ kind: "excel", sheet: "数据" })).toBe("Excel · 工作表「数据」");
  });

  it("defaults the match key to the primary key, or the first fully mapped unique key for upsert", () => {
    expect(defaultKeys("append", keys, mapping("id"))).toEqual([]);
    expect(defaultKeys("update", keys, mapping("id", "name"))).toEqual(["id"]);
    expect(defaultKeys("delete", keys, mapping("name"))).toEqual([]);
    // 复合主键只映射了一部分:不默认,免得按半个主键删掉一大片。
    const composite: UniqueKey[] = [{ name: null, columns: ["order_id", "line_no"] }];
    expect(defaultKeys("delete", composite, mapping("order_id", "qty"))).toEqual([]);
    expect(defaultKeys("delete", composite, mapping("order_id", "line_no"))).toEqual(["order_id", "line_no"]);
    expect(defaultKeys("upsert", keys, mapping("code", "region", "name"))).toEqual(["code", "region"]);
    expect(defaultKeys("upsert", [], mapping("id"))).toEqual([]);
  });

  it("explains what is wrong with the chosen match keys", () => {
    expect(keyProblem("append", [], keys, mapping("id"))).toBeNull();
    expect(keyProblem("update", [], keys, mapping("id", "name"))).toContain("要勾选匹配键");
    expect(keyProblem("delete", ["id"], keys, mapping("name"))).toBe("匹配键「id」没有选来源列。");
    expect(keyProblem("upsert", ["code"], keys, mapping("code", "region"))).toContain("只能按主键或一个唯一索引");
    expect(keyProblem("upsert", ["region", "code"], keys, mapping("code", "region"))).toBeNull();
    expect(keyProblem("update", ["id"], keys, mapping("id"))).toContain("至少还要给一列");
    expect(keyProblem("update", ["id"], keys, mapping("id", "name"))).toBeNull();
    expect(keyProblem("delete", ["name"], keys, mapping("name"))).toBeNull();
  });

  it("describes keys, confirmations and counts per mode", () => {
    expect(keys.map(describeKey)).toEqual(["主键（id）", "唯一索引 uk_code（code、region）"]);
    expect(confirmation("append", "shop.users", "mysql")).toBeNull();
    expect(confirmation("replace", "shop.users", "mysql")?.action).toBe("清空并导入");
    // MySQL 的「复制」整体一个事务,不按批提交;删除仍按批提交。
    expect(confirmation("replace", "shop.users", "mysql")?.text).toContain("同一个事务");
    expect(confirmation("delete", "shop.users", "mariadb")?.text).toContain("每批单独提交");
    expect(confirmation("delete", "shop.users", "sqlite")?.text).toContain("「shop.users」");
    expect(commitNote("postgres", "replace")).toContain("全部回滚");
    expect([isDestructive("replace"), isDestructive("delete"), isDestructive("upsert")]).toEqual([true, true, false]);
    expect((["append", "update", "upsert", "delete", "replace"] as const).map(affectedLabel)).toEqual([
      "插入",
      "更新",
      "处理",
      "删除",
      "插入",
    ]);
  });

  it("asks to choose a sheet only when a workbook has several", () => {
    const excel = (sheets: string[]): SourcePreview => ({ ...preview, format: { kind: "excel", sheet: null }, sheets });
    expect(needsSheetChoice(excel(["Sheet1", "说明"]))).toBe(true);
    expect(needsSheetChoice(excel(["Sheet1"]))).toBe(false);
    expect(needsSheetChoice(preview)).toBe(false);
  });

  it("warns that MySQL upserts on every unique key", () => {
    expect(mysqlUpsertNote("mysql", "upsert", keys)).toContain("看所有唯一键");
    expect(mysqlUpsertNote("mysql", "upsert", keys.slice(0, 1))).toBeNull();
    expect(mysqlUpsertNote("postgres", "upsert", keys)).toBeNull();
    expect(mysqlUpsertNote("mariadb", "update", keys)).toBeNull();
  });

  it("titles and tones follow the report", () => {
    expect([reportTone(report({})), reportTitle(report({}))]).toEqual(["success", "导入完成"]);
    expect(reportTone(report({ failed: 2 }))).toBe("warning");
    expect(reportTitle(report({ failed: 2 }))).toContain("失败");
    expect([reportTone(report({ status: "stopped" })), reportTitle(report({ status: "stopped" }))]).toEqual([
      "error",
      "遇到错误，已停止",
    ]);
    expect(reportTone(report({ status: "cancelled" }))).toBe("warning");
    expect(reportTitle(report({ status: "cancelled" }))).toBe("已取消");
    expect(reportTitle(report({ status: "failed" }))).toBe("导入中途出错");
    expect(failuresFileName({ database: "shop", schema: null, table: "users" })).toBe("users-导入失败.csv");
    expect(commitNote("mariadb")).toContain("每批单独提交");
    expect(commitNote("sqlite")).toContain("一个事务");
  });
});
