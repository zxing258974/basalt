import { describe, expect, it } from "vitest";
import type { CellValue } from "../../lib/types";
import { MAX_DISPLAY_CHARS, displayText, editorText, isNumericCell, sameAsOriginal } from "./cellFormat";

const text = (value: string): CellValue => ({ kind: "text", value });

describe("displayText", () => {
  it("shows NULL, booleans and numbers as they came", () => {
    expect(displayText({ kind: "null" })).toBe("NULL");
    expect(displayText({ kind: "bool", value: false })).toBe("false");
    expect(displayText({ kind: "int", value: "9007199254740993" })).toBe("9007199254740993");
    expect(displayText({ kind: "decimal", value: "12.50" })).toBe("12.50");
  });

  it("keeps one line per row and shortens very long text", () => {
    expect(displayText(text("第一行\n第二行\r\n三"))).toBe("第一行↵第二行↵三");
    const long = displayText(text("x".repeat(MAX_DISPLAY_CHARS + 50)));
    expect(long).toHaveLength(MAX_DISPLAY_CHARS + 1);
    expect(long.endsWith("…")).toBe(true);
  });

  it("shows bytes as 0x hex and names unsupported types", () => {
    expect(displayText({ kind: "bytes", value: "00ff" })).toBe("0x00ff");
    expect(displayText({ kind: "bytes", value: "" })).toBe("0x");
    expect(displayText({ kind: "unsupported", value: { typeName: "GEOMETRY" } })).toBe("‹GEOMETRY›");
  });
});

describe("editorText", () => {
  it("gives the text a user edits, keeping newlines", () => {
    expect(editorText({ kind: "null" })).toBeNull();
    expect(editorText({ kind: "bool", value: true })).toBe("true");
    expect(editorText({ kind: "bytes", value: "0a1b" })).toBe("0x0a1b");
    expect(editorText(text("a\nb"))).toBe("a\nb");
    expect(editorText({ kind: "dateTime", value: "2026-10-20 08:00:00" })).toBe("2026-10-20 08:00:00");
  });

  it("has nothing to edit for unsupported values", () => {
    expect(editorText({ kind: "unsupported", value: { typeName: "GEOMETRY" } })).toBeUndefined();
  });
});

describe("sameAsOriginal", () => {
  it("compares the edited text with the original's editor text", () => {
    expect(sameAsOriginal(text("Ann"), "Ann")).toBe(true);
    expect(sameAsOriginal(text("Ann"), "Anne")).toBe(false);
    expect(sameAsOriginal({ kind: "null" }, null)).toBe(true);
    expect(sameAsOriginal({ kind: "null" }, "")).toBe(false);
    expect(sameAsOriginal(text(""), null)).toBe(false);
  });
});

describe("isNumericCell", () => {
  it("right-aligns numbers only", () => {
    expect(isNumericCell({ kind: "uint", value: "1" })).toBe(true);
    expect(isNumericCell({ kind: "float", value: "0.5" })).toBe(true);
    expect(isNumericCell(text("1"))).toBe(false);
    expect(isNumericCell({ kind: "null" })).toBe(false);
  });
});
