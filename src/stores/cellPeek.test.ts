import { beforeEach, describe, expect, it } from "vitest";
import { samePeek, useCellPeek, type CellPeek } from "./cellPeek";

const peek = (text: string): CellPeek => ({
  source: "orders",
  column: "shipping",
  dataType: "json",
  rowNumber: 2,
  value: { kind: "text", text },
  edited: false,
  deleted: false,
});

describe("cell peek store", () => {
  beforeEach(() => useCellPeek.setState({ tabId: null, owner: null, peek: null }));

  it("ignores a report that shows the same thing again", () => {
    useCellPeek.getState().publish("t1", "grid-a", peek("{}"));
    const before = useCellPeek.getState();
    useCellPeek.getState().publish("t1", "grid-a", peek("{}"));
    expect(useCellPeek.getState()).toBe(before);
    useCellPeek.getState().publish("t1", "grid-a", peek("[]"));
    expect(useCellPeek.getState().peek?.value).toEqual({ kind: "text", text: "[]" });
  });

  it("only lets the grid that reported clear it", () => {
    useCellPeek.getState().publish("t1", "grid-b", peek("{}"));
    useCellPeek.getState().release("grid-a");
    expect(useCellPeek.getState().owner).toBe("grid-b");
    useCellPeek.getState().release("grid-b");
    expect(useCellPeek.getState()).toMatchObject({ tabId: null, owner: null, peek: null });
  });

  it("compares every field that shows", () => {
    expect(samePeek(null, null)).toBe(true);
    expect(samePeek(peek("{}"), null)).toBe(false);
    expect(samePeek(peek("{}"), { ...peek("{}"), edited: true })).toBe(false);
    expect(samePeek(peek("{}"), { ...peek("{}"), deleted: true })).toBe(false);
    expect(samePeek(peek("{}"), { ...peek("{}"), value: { kind: "null" } })).toBe(false);
    expect(
      samePeek(
        { ...peek(""), value: { kind: "bytes", hex: "00" } },
        { ...peek(""), value: { kind: "bytes", hex: "00" } },
      ),
    ).toBe(true);
  });
});
