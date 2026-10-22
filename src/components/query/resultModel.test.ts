import { describe, expect, it } from "vitest";
import type { ResultSet } from "../../lib/queryTypes";
import type { CellValue } from "../../lib/types";
import { batchSize, resultPage, sortKey, toResultSort } from "./resultModel";

const int = (n: number): CellValue => ({ kind: "int", value: String(n) });

describe("resultPage", () => {
  const set: ResultSet = {
    id: 1,
    columns: [
      { name: "id", dataType: "int", source: null },
      { name: "id", dataType: "varchar", source: null },
    ],
    rows: [[int(1), int(2)]],
    totalRows: 1,
    truncated: false,
    readOnly: null,
  };

  it("wraps loaded rows as a read-only page", () => {
    const rows = [[int(3), int(4)]];
    const page = resultPage(set, rows, null, "只读");
    expect(page.rows).toBe(rows);
    expect(page.readOnlyReason).toBe("只读");
    expect(page.columns.map((c) => [c.name, c.dataType, c.isPrimaryKey, c.isGenerated])).toEqual([
      // 同名的列在表头上要分得开。
      ["id", "int", false, false],
      ["id #2", "varchar", false, false],
    ]);
  });

  it("uses the editable columns when there are some", () => {
    const columns = resultPage(set, [], null, null).columns.map((c) => ({ ...c, isPrimaryKey: true }));
    expect(resultPage(set, [], columns, null).columns).toBe(columns);
  });
});

describe("sorting and batches", () => {
  it("sorts by column index so same-named columns stay apart", () => {
    expect(toResultSort({ column: sortKey(1, "id"), descending: true })).toEqual({ column: 1, descending: true });
    expect(toResultSort(null)).toBeNull();
    expect(toResultSort({ column: "x:id", descending: false })).toBeNull();
  });

  it("grows batches as more rows are loaded, within the backend's limit", () => {
    expect(batchSize(0)).toBe(1000);
    expect(batchSize(8000)).toBe(2000);
    expect(batchSize(1_000_000)).toBe(5000);
  });
});
