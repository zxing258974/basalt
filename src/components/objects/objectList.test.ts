import { describe, expect, it } from "vitest";
import type { TableInfo } from "../../lib/types";
import { filterTables, sortTables, visibleColumns } from "./objectList";

function t(name: string, partial: Partial<TableInfo> = {}): TableInfo {
  return {
    name,
    kind: "table",
    engine: null,
    rows: null,
    dataBytes: null,
    indexBytes: null,
    createdAt: null,
    updatedAt: null,
    comment: null,
    ...partial,
  };
}

const tables = [
  t("orders", { rows: 900, dataBytes: 4096, comment: "订单" }),
  t("Customers", { rows: 50, dataBytes: 9000 }),
  t("v_sales", { kind: "view" }),
  t("totals", { kind: "materializedView", rows: 1 }),
];

describe("filterTables", () => {
  it("filters by kind, treating every non-table relation as a view", () => {
    expect(filterTables(tables, { kind: "tables", search: "" }).map((x) => x.name)).toEqual(["orders", "Customers"]);
    expect(filterTables(tables, { kind: "views", search: "" }).map((x) => x.name)).toEqual(["v_sales", "totals"]);
    expect(filterTables(tables, { kind: "all", search: "" })).toHaveLength(4);
  });

  it("matches name or comment case-insensitively", () => {
    expect(filterTables(tables, { kind: "all", search: "CUST" }).map((x) => x.name)).toEqual(["Customers"]);
    expect(filterTables(tables, { kind: "all", search: "订单" }).map((x) => x.name)).toEqual(["orders"]);
    expect(filterTables(tables, { kind: "all", search: "  " })).toHaveLength(4);
  });
});

describe("sortTables", () => {
  it("sorts names naturally and case-insensitively without mutating", () => {
    const input = [t("t10"), t("t2"), t("B"), t("a")];
    expect(sortTables(input, { key: "name", direction: "asc" }).map((x) => x.name)).toEqual(["a", "B", "t2", "t10"]);
    expect(input.map((x) => x.name)).toEqual(["t10", "t2", "B", "a"]);
  });

  it("sorts numbers descending with unknown values always last", () => {
    const sorted = sortTables(tables, { key: "rows", direction: "desc" }).map((x) => x.name);
    expect(sorted).toEqual(["orders", "Customers", "totals", "v_sales"]);
    const asc = sortTables(tables, { key: "rows", direction: "asc" }).map((x) => x.name);
    expect(asc).toEqual(["totals", "Customers", "orders", "v_sales"]);
  });
});

describe("visibleColumns", () => {
  it("shows only optional columns that carry data somewhere", () => {
    expect(visibleColumns(tables)).toEqual(["rows", "dataBytes", "comment"]);
    expect(visibleColumns([t("notes")])).toEqual([]);
  });
});
