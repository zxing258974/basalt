import { describe, expect, it } from "vitest";
import type { CellValue } from "../types";
import { compareCells, createMockQueries } from "./queryMock";

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

describe("mock results", () => {
  const scope = { database: "shop", schema: null };
  const queries = () =>
    createMockQueries(
      () => [],
      async () => {},
    );

  it("keeps the whole result and hands it out in batches, sorted if asked", async () => {
    const mock = queries();
    const outcome = await mock.runStatement("t", "c", scope, "SELECT * FROM orders");
    const set = outcome.resultSets[0]!;
    expect(set.rows.length).toBe(Math.min(1000, set.totalRows));
    expect(set.totalRows).toBeGreaterThan(1000);
    const rest = await mock.resultRows("t", set.id, 1000, 5000, null);
    expect(rest.length).toBe(Math.min(5000, set.totalRows - 1000));
    const sorted = await mock.resultRows("t", set.id, 0, 3, { column: 0, descending: true });
    expect(compareCells(sorted[0]![0]!, sorted[1]![0]!)).toBeGreaterThanOrEqual(0);

    await mock.releaseResults("t", [set.id]);
    await expect(mock.resultRows("t", set.id, 0, 10, null)).rejects.toMatchObject({ code: "not_found" });
  });

  it("says which table each column comes from", async () => {
    const outcome = await queryMockSelect("SELECT id, name AS who, 1 + 1 AS two FROM customers");
    expect(outcome.columns.map((c) => [c.name, c.source?.column ?? null])).toEqual([
      ["id", "id"],
      ["who", "name"],
      ["two", null],
    ]);
    expect(outcome.columns[0]!.source!.table).toEqual({ database: "shop", schema: null, table: "customers" });
  });

  async function queryMockSelect(sql: string) {
    const outcome = await queries().runStatement("t", "c", scope, sql);
    return outcome.resultSets[0]!;
  }
});
