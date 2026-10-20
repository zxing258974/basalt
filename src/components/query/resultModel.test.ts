import { describe, expect, it } from "vitest";
import type { CellValue } from "../../lib/types";
import { compareCells, RESULT_READ_ONLY, resultPage, sortKey, sortRows } from "./resultModel";

const int = (n: number): CellValue => ({ kind: "int", value: String(n) });
const text = (value: string): CellValue => ({ kind: "text", value });
const NULL: CellValue = { kind: "null" };

describe("compareCells", () => {
  it("puts NULL first and compares numbers by value", () => {
    expect(compareCells(NULL, int(1))).toBe(-1);
    expect(compareCells(int(1), NULL)).toBe(1);
    expect(compareCells(NULL, NULL)).toBe(0);
    expect(compareCells(int(9), int(10))).toBe(-1);
    expect(compareCells({ kind: "decimal", value: "2.50" }, { kind: "uint", value: "2" })).toBe(1);
  });

  it("compares text naturally and treats odd values as text", () => {
    expect(compareCells(text("a2"), text("a10"))).toBeLessThan(0);
    expect(compareCells(text("北京"), text("上海"))).toBeLessThan(0);
    expect(compareCells({ kind: "bool", value: true }, { kind: "bool", value: false })).toBeGreaterThan(0);
    expect(compareCells({ kind: "unsupported", value: { typeName: "x" } }, text("a"))).toBeLessThan(0);
  });
});

describe("sortRows and resultPage", () => {
  const set = {
    columns: [
      { name: "id", dataType: "int" },
      { name: "id", dataType: "varchar" },
    ],
    rows: [
      [int(2), text("b")],
      [int(1), text("c")],
      [int(2), text("a")],
    ],
    truncated: false,
  };

  it("sorts by column index, stably, in either direction", () => {
    expect(sortRows(set.rows, 0, false).map((r) => r[1])).toEqual([text("c"), text("b"), text("a")]);
    expect(sortRows(set.rows, 0, true).map((r) => r[1])).toEqual([text("b"), text("a"), text("c")]);
  });

  it("wraps a result set as a read-only page and tells same-named columns apart", () => {
    const unsorted = resultPage(set, null);
    expect(unsorted.rows).toBe(set.rows);
    expect(unsorted.readOnlyReason).toBe(RESULT_READ_ONLY);
    expect(unsorted.columns.map((c) => [c.name, c.dataType, c.isPrimaryKey])).toEqual([
      ["id", "int", false],
      ["id", "varchar", false],
    ]);
    const sorted = resultPage(set, { column: sortKey(1, "id"), descending: false });
    expect(sorted.rows.map((r) => r[1])).toEqual([text("a"), text("b"), text("c")]);
    expect(resultPage(set, { column: "9:nope", descending: false }).rows).toBe(set.rows);
  });
});
