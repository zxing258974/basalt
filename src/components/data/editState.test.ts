import { describe, expect, it } from "vitest";
import type { CellValue, ColumnInfo, TablePage } from "../../lib/types";
import {
  EMPTY_EDITS,
  addRow,
  buildChanges,
  deleteRow,
  editedValue,
  pendingCount,
  removeNewRow,
  restoreRow,
  revertCell,
  setCell,
} from "./editState";

function column(name: string, partial: Partial<ColumnInfo> = {}): ColumnInfo {
  return {
    name,
    dataType: "text",
    isPrimaryKey: false,
    nullable: true,
    hasDefault: false,
    isGenerated: false,
    valueType: { kind: "text" },
    ...partial,
  };
}

const int = (value: number): CellValue => ({ kind: "int", value: String(value) });
const text = (value: string): CellValue => ({ kind: "text", value });

const page: TablePage = {
  columns: [column("id", { isPrimaryKey: true, nullable: false }), column("name"), column("note")],
  rows: [
    [int(1), text("Ann"), { kind: "null" }],
    [int(2), text("Ben"), text("x")],
  ],
  hasMore: false,
  readOnlyReason: null,
  sql: "SELECT …",
};

const row = (index: number) => ({ kind: "existing", index }) as const;

describe("edit state", () => {
  it("records an edit and forgets it once the original value is typed back", () => {
    const edited = setCell(EMPTY_EDITS, page, row(0), "name", "Anne");
    expect(editedValue(edited, row(0), "name")).toBe("Anne");
    expect(pendingCount(edited)).toBe(1);

    const back = setCell(edited, page, row(0), "name", "Ann");
    expect(editedValue(back, row(0), "name")).toBeUndefined();
    expect(pendingCount(back)).toBe(0);
    expect(back.edits).toEqual({});
  });

  it("keeps NULL and the empty string apart", () => {
    const empty = setCell(EMPTY_EDITS, page, row(0), "note", "");
    expect(editedValue(empty, row(0), "note")).toBe("");
    const nulled = setCell(empty, page, row(0), "note", null);
    expect(pendingCount(nulled)).toBe(0);
    expect(editedValue(setCell(EMPTY_EDITS, page, row(1), "note", null), row(1), "note")).toBeNull();
  });

  it("never mutates the previous state", () => {
    const first = setCell(EMPTY_EDITS, page, row(0), "name", "A");
    const second = setCell(first, page, row(0), "note", "B");
    expect(first.edits[0]).toEqual({ name: "A" });
    expect(second.edits[0]).toEqual({ name: "A", note: "B" });
    expect(EMPTY_EDITS.edits).toEqual({});
  });

  it("reverts a single cell", () => {
    const edited = setCell(setCell(EMPTY_EDITS, page, row(0), "name", "A"), page, row(0), "note", "B");
    const reverted = revertCell(edited, row(0), "name");
    expect(reverted.edits[0]).toEqual({ note: "B" });
    expect(revertCell(reverted, row(0), "note").edits).toEqual({});
  });

  it("deleting a row drops its edits, restoring brings back the original", () => {
    const edited = setCell(EMPTY_EDITS, page, row(1), "name", "Bea");
    const deleted = deleteRow(edited, 1);
    expect(deleted.deleted).toEqual([1]);
    expect(deleted.edits).toEqual({});
    expect(pendingCount(deleted)).toBe(1);
    expect(deleteRow(deleted, 1)).toBe(deleted);
    expect(restoreRow(deleted, 1).deleted).toEqual([]);
  });

  it("new rows keep only the cells the user filled and can be discarded", () => {
    const withRow = addRow(addRow(EMPTY_EDITS));
    expect(withRow.inserted.map((r) => r.id)).toEqual([0, 1]);
    const filled = setCell(withRow, page, { kind: "new", id: 1 }, "name", "Cat");
    expect(filled.inserted[1]?.values).toEqual({ name: "Cat" });
    expect(editedValue(filled, { kind: "new", id: 1 }, "name")).toBe("Cat");
    const cleared = revertCell(filled, { kind: "new", id: 1 }, "name");
    expect(cleared.inserted[1]?.values).toEqual({});
    const removed = removeNewRow(filled, 0);
    expect(removed.inserted.map((r) => r.id)).toEqual([1]);
    expect(pendingCount(removed)).toBe(1);
  });

  it("builds changes keyed by the original primary key", () => {
    let state = setCell(EMPTY_EDITS, page, row(0), "id", "10");
    state = setCell(state, page, row(0), "note", "");
    state = setCell(state, page, row(0), "name", null);
    state = deleteRow(state, 1);
    state = addRow(state);
    state = setCell(state, page, { kind: "new", id: 0 }, "name", "Cat");
    state = addRow(state);

    expect(buildChanges(state, page)).toEqual([
      { kind: "update", key: { id: int(1) }, values: { id: "10", note: "", name: null } },
      { kind: "delete", key: { id: int(2) } },
      { kind: "insert", values: { name: "Cat" } },
      { kind: "insert", values: {} },
    ]);
  });
});
