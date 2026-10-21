import { describe, expect, it } from "vitest";
import type { ImportReport, SourcePreview } from "../../lib/transferTypes";
import type { ColumnInfo } from "../../lib/types";
import {
  commitNote,
  describeFormat,
  failuresFileName,
  mappedCount,
  mappedPreview,
  missingRequired,
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
  return { status: "completed", inserted: 1, failed: 0, failures: [], summary: "", ...partial };
}

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
