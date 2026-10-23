import { describe, expect, it } from "vitest";
import type { CellValue, ColumnInfo, TablePage } from "../../lib/types";
import { addRow, deleteRow, EMPTY_EDITS, setCell } from "../data/editState";
import { gridRows } from "../data/gridModel";
import { jsonTokens, MAX_FORMAT_CHARS, peekCell, prettyJson, textStats } from "./cellPeekModel";

function column(name: string, dataType: string, extra: Partial<ColumnInfo> = {}): ColumnInfo {
  return {
    name,
    dataType,
    isPrimaryKey: false,
    nullable: true,
    hasDefault: false,
    isGenerated: false,
    valueType: { kind: "text" },
    ...extra,
  };
}

const page: TablePage = {
  columns: [
    column("id", "int", { isPrimaryKey: true, nullable: false, valueType: { kind: "integer", unsigned: false } }),
    column("payload", "text"),
    column("raw", "blob", { valueType: { kind: "binary" } }),
    column("shape", "geometry"),
    column("created", "datetime", { hasDefault: true }),
  ],
  rows: [
    [
      { kind: "int", value: "7" },
      { kind: "text", value: '{"a":1}' },
      { kind: "bytes", value: "00ff" },
      { kind: "unsupported", value: { typeName: "GEOMETRY" } },
      { kind: "null" },
    ] satisfies CellValue[],
    [
      { kind: "int", value: "8" },
      { kind: "bool", value: true },
      { kind: "null" },
      { kind: "null" },
      { kind: "dateTime", value: "2026-10-20 08:05:09.120" },
    ] satisfies CellValue[],
  ],
  hasMore: false,
  readOnlyReason: null,
  sql: "",
};

const where = { source: "events", rowOffset: 100 };

describe("peekCell", () => {
  it("shows the stored value with where it comes from", () => {
    const rows = gridRows(page, EMPTY_EDITS);
    expect(peekCell(page, EMPTY_EDITS, rows, { row: 0, col: 1 }, where)).toEqual({
      source: "events",
      column: "payload",
      dataType: "text",
      rowNumber: 101,
      value: { kind: "text", text: '{"a":1}' },
      edited: false,
      deleted: false,
    });
    const value = (row: number, col: number) => peekCell(page, EMPTY_EDITS, rows, { row, col }, where)?.value;
    expect(value(0, 2)).toEqual({ kind: "bytes", hex: "00ff" });
    expect(value(0, 3)).toEqual({ kind: "unsupported", typeName: "GEOMETRY" });
    expect(value(0, 4)).toEqual({ kind: "null" });
    expect(value(1, 1)).toEqual({ kind: "text", text: "true" });
    expect(value(1, 4)).toEqual({ kind: "text", text: "2026-10-20 08:05:09.120" });
  });

  it("shows the pending value of an edited cell", () => {
    const edits = setCell(EMPTY_EDITS, page, { kind: "existing", index: 0 }, "payload", null);
    const peek = peekCell(page, edits, gridRows(page, edits), { row: 0, col: 1 }, where);
    expect(peek?.value).toEqual({ kind: "null" });
    expect(peek?.edited).toBe(true);
  });

  it("shows hex typed into a binary column as bytes, like it will be written", () => {
    const edits = setCell(EMPTY_EDITS, page, { kind: "existing", index: 0 }, "raw", "0xABCD");
    expect(peekCell(page, edits, gridRows(page, edits), { row: 0, col: 2 }, where)?.value).toEqual({
      kind: "bytes",
      hex: "ABCD",
    });
    const text = setCell(EMPTY_EDITS, page, { kind: "existing", index: 0 }, "payload", "0xABCD");
    expect(peekCell(page, text, gridRows(page, text), { row: 0, col: 1 }, where)?.value).toEqual({
      kind: "text",
      text: "0xABCD",
    });
  });

  it("says when the row is marked for deletion", () => {
    const edits = deleteRow(EMPTY_EDITS, 1);
    expect(peekCell(page, edits, gridRows(page, edits), { row: 1, col: 1 }, where)?.deleted).toBe(true);
  });

  it("explains what an unfilled cell of a new row becomes", () => {
    const edits = addRow(EMPTY_EDITS);
    const rows = gridRows(page, edits);
    const peek = peekCell(page, edits, rows, { row: 2, col: 4 }, where);
    expect(peek?.rowNumber).toBeNull();
    expect(peek?.value).toEqual({ kind: "unset", hint: "默认" });
  });

  it("is empty without a selection or outside the grid", () => {
    const rows = gridRows(page, EMPTY_EDITS);
    expect(peekCell(page, EMPTY_EDITS, rows, null, where)).toBeNull();
    expect(peekCell(page, EMPTY_EDITS, rows, { row: 9, col: 0 }, where)).toBeNull();
  });
});

describe("prettyJson", () => {
  it("indents objects and arrays and keeps empty ones on one line", () => {
    expect(prettyJson('{"a":1,"b":[true,null],"c":{},"d":[]}')).toBe(
      '{\n  "a": 1,\n  "b": [\n    true,\n    null\n  ],\n  "c": {},\n  "d": []\n}',
    );
  });

  it("copies numbers and strings exactly as written", () => {
    expect(prettyJson('{"big":9007199254740993,"f":1.0,"e":1E3}')).toBe(
      '{\n  "big": 9007199254740993,\n  "f": 1.0,\n  "e": 1E3\n}',
    );
    expect(prettyJson('["a,\\"b\\":{c}", "\\\\", "\\u4e2d"]')).toBe('[\n  "a,\\"b\\":{c}",\n  "\\\\",\n  "\\u4e2d"\n]');
  });

  it("re-indents JSON that is already spread over lines", () => {
    expect(prettyJson('\n{ "a" :\n\t[ 1 , 2 ] }\n')).toBe('{\n  "a": [\n    1,\n    2\n  ]\n}');
  });

  it("gives up on values too long or nested too deep to lay out quickly", () => {
    expect(prettyJson(`[${"1,".repeat(MAX_FORMAT_CHARS / 2)}1]`)).toBeNull();
    expect(prettyJson(`${"[".repeat(101)}1${"]".repeat(101)}`)).toBeNull();
    expect(prettyJson(`${"[".repeat(100)}1${"]".repeat(100)}`)).toContain("\n" + "  ".repeat(100) + "1");
    expect(prettyJson(`${"[".repeat(20_000)}1${"]".repeat(20_000)}`)).toBeNull();
  });

  it("only strips the whitespace JSON allows, so a BOM is not silently dropped", () => {
    expect(prettyJson('\uFEFF{"a":1}')).toBeNull();
    expect(prettyJson(' \t{"a":1}\r\n')).toBe('{\n  "a": 1\n}');
  });

  it("leaves everything that is not a JSON object or array alone", () => {
    expect(prettyJson("hello")).toBeNull();
    expect(prettyJson('"just a string"')).toBeNull();
    expect(prettyJson("42")).toBeNull();
    expect(prettyJson("{not json}")).toBeNull();
    expect(prettyJson("[1, 2")).toBeNull();
  });
});

describe("jsonTokens", () => {
  it("tells keys from string values and numbers from literals", () => {
    const tokens = jsonTokens(prettyJson('{"k":"v","n":-1.5e3,"t":true,"z":null}')!);
    const kinds = tokens.filter((t) => t.kind !== "space" && t.kind !== "punct").map((t) => [t.kind, t.text]);
    expect(kinds).toEqual([
      ["key", '"k"'],
      ["string", '"v"'],
      ["key", '"n"'],
      ["number", "-1.5e3"],
      ["key", '"t"'],
      ["literal", "true"],
      ["key", '"z"'],
      ["literal", "null"],
    ]);
    expect(tokens.map((t) => t.text).join("")).toBe(prettyJson('{"k":"v","n":-1.5e3,"t":true,"z":null}'));
  });

  it("does not end a string at an escaped quote", () => {
    expect(jsonTokens('["a\\"b"]').map((t) => t.kind)).toEqual(["punct", "string", "punct"]);
  });
});

describe("textStats", () => {
  it("counts characters, not UTF-16 units, and lines", () => {
    expect(textStats("")).toEqual({ chars: 0, lines: 1 });
    expect(textStats("中文😀\nab")).toEqual({ chars: 6, lines: 2 });
  });
});
