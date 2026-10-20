import { describe, expect, it } from "vitest";
import type { CellValue, ColumnInfo, TablePage } from "../../lib/types";
import { EMPTY_EDITS, addRow, deleteRow, setCell } from "./editState";
import { MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, cellView, gridRows, initialWidth } from "./gridModel";

function column(name: string, partial: Partial<ColumnInfo> = {}): ColumnInfo {
  return { name, dataType: "text", isPrimaryKey: false, nullable: true, hasDefault: false, isGenerated: false, valueType: { kind: "text" }, ...partial };
}

const text = (value: string): CellValue => ({ kind: "text", value });

const page: TablePage = {
  columns: [
    column("id", { isPrimaryKey: true, nullable: false, hasDefault: true, dataType: "int" }),
    column("name", { nullable: false }),
    column("note"),
    column("shape", { dataType: "geometry" }),
    column("label", { isGenerated: true }),
  ],
  rows: [
    [{ kind: "int", value: "1" }, text("Ann"), { kind: "null" }, { kind: "unsupported", value: { typeName: "GEOMETRY" } }, text("#Ann")],
    [{ kind: "int", value: "2" }, text("Ben"), text("x"), { kind: "null" }, text("#Ben")],
  ],
  hasMore: false,
  readOnlyReason: null,
  sql: "",
};

describe("gridRows", () => {
  it("lists page rows, then new rows, and marks deleted ones", () => {
    const state = addRow(deleteRow(EMPTY_EDITS, 1));
    const rows = gridRows(page, state);
    expect(rows.map((r) => r.target)).toEqual([
      { kind: "existing", index: 0 },
      { kind: "existing", index: 1 },
      { kind: "new", id: 0 },
    ]);
    expect(rows.map((r) => r.status)).toEqual(["clean", "deleted", "new"]);
    expect(gridRows(page, setCell(EMPTY_EDITS, page, { kind: "existing", index: 0 }, "name", "A"))[0]?.status).toBe("edited");
  });
});

describe("cellView", () => {
  const [first, second] = gridRows(page, EMPTY_EDITS);

  it("shows original values with their kind", () => {
    expect(cellView(page, EMPTY_EDITS, first!, 1)).toMatchObject({ text: "Ann", tone: "value", edited: false, editable: true });
    expect(cellView(page, EMPTY_EDITS, first!, 2)).toMatchObject({ text: "NULL", tone: "null" });
    expect(cellView(page, EMPTY_EDITS, first!, 0)).toMatchObject({ numeric: true });
  });

  it("locks unsupported values, generated columns and deleted rows", () => {
    expect(cellView(page, EMPTY_EDITS, first!, 3)).toMatchObject({ text: "‹GEOMETRY›", tone: "unsupported", editable: false });
    // 同一列别的行是 NULL 时可以编辑。
    expect(cellView(page, EMPTY_EDITS, second!, 3).editable).toBe(true);
    expect(cellView(page, EMPTY_EDITS, first!, 4).editable).toBe(false);
    const deleted = deleteRow(EMPTY_EDITS, 0);
    expect(cellView(page, deleted, gridRows(page, deleted)[0]!, 1).editable).toBe(false);
  });

  it("locks binary values that sit in a text column", () => {
    const mixed: TablePage = {
      ...page,
      columns: [page.columns[0]!, column("blob", { valueType: { kind: "binary" } }), column("note")],
      rows: [[{ kind: "int", value: "1" }, { kind: "bytes", value: "0a1b" }, { kind: "bytes", value: "ff" }]],
    };
    const row = gridRows(mixed, EMPTY_EDITS)[0]!;
    expect(cellView(mixed, EMPTY_EDITS, row, 1)).toMatchObject({ text: "0x0a1b", editable: true, editorText: "0x0a1b" });
    expect(cellView(mixed, EMPTY_EDITS, row, 2)).toMatchObject({ text: "0xff", editable: false });
  });

  it("shows pending edits and editor text", () => {
    const state = setCell(EMPTY_EDITS, page, { kind: "existing", index: 0 }, "note", "第一行\n第二行");
    const view = cellView(page, state, gridRows(page, state)[0]!, 2);
    expect(view).toMatchObject({ text: "第一行↵第二行", tone: "value", edited: true, editorText: "第一行\n第二行" });
  });

  it("explains what an empty cell of a new row will become", () => {
    const state = addRow(EMPTY_EDITS);
    const row = gridRows(page, state)[2]!;
    expect(cellView(page, state, row, 0)).toMatchObject({ text: "默认", tone: "placeholder", editorText: "" });
    expect(cellView(page, state, row, 1)).toMatchObject({ text: "必填", tone: "placeholder" });
    expect(cellView(page, state, row, 2)).toMatchObject({ text: "NULL", tone: "placeholder" });
    expect(cellView(page, state, row, 4)).toMatchObject({ text: "自动生成", editable: false });
  });
});

describe("initialWidth", () => {
  it("fits the header and a sample of values within bounds", () => {
    const narrow = initialWidth(column("id", { dataType: "int" }), [{ kind: "int", value: "1" }]);
    expect(narrow).toBeGreaterThanOrEqual(MIN_COLUMN_WIDTH);
    const wide = initialWidth(column("body"), [text("很长".repeat(200))]);
    expect(wide).toBe(MAX_COLUMN_WIDTH);
    expect(initialWidth(column("email"), [text("user42@example.com")])).toBeGreaterThan(narrow);
  });
});
