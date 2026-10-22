import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { describe, expect, it } from "vitest";
import { errorMarkField, markRange, showErrorMark } from "./errorMark";

const state = (doc: string) => EditorState.create({ doc, extensions: [errorMarkField] });

function marks(view: EditorView): [number, number][] {
  const found: [number, number][] = [];
  view.state.field(errorMarkField).between(0, view.state.doc.length, (from, to) => {
    found.push([from, to]);
  });
  return found;
}

describe("markRange", () => {
  it("covers the word at the error", () => {
    expect(markRange(state("SELECT * FORM t"), 9)).toEqual({ from: 9, to: 13 });
    expect(markRange(state("SELECT 中文名 FROM t"), 7)).toEqual({ from: 7, to: 10 });
  });

  it("marks one character on punctuation", () => {
    expect(markRange(state("SELECT a,, b"), 9)).toEqual({ from: 9, to: 10 });
  });

  it("steps back onto the last word when the error is at the end or on space", () => {
    expect(markRange(state("SELECT * FROM"), 13)).toEqual({ from: 9, to: 13 });
    expect(markRange(state("SELECT"), 99)).toEqual({ from: 0, to: 6 });
    // 后面还有别的语句:不标到下一条的分号、换行上。
    expect(markRange(state("SELECT * FROM;\nSELECT 1"), 13, 13)).toEqual({ from: 9, to: 13 });
    expect(markRange(state("SELECT a,\n"), 9, 9)).toEqual({ from: 8, to: 9 });
    expect(markRange(state(""), 0)).toBeNull();
    expect(markRange(state("   "), 1)).toBeNull();
  });

  it("does not split a character made of two UTF-16 units", () => {
    expect(markRange(state("SELECT 😀 x"), 7)).toEqual({ from: 7, to: 9 });
    expect(markRange(state("SELECT 😀"), 9)).toEqual({ from: 7, to: 9 });
  });
});

describe("error mark", () => {
  it("follows edits elsewhere and goes away when its text is edited", () => {
    const view = new EditorView({ state: state("SELECT * FORM t") });
    showErrorMark(view, { at: 9, message: "near FORM" });
    expect(marks(view)).toEqual([[9, 13]]);
    expect(view.contentDOM.querySelector(".cm-sql-error")?.getAttribute("title")).toBe("near FORM");

    view.dispatch({ changes: { from: 0, insert: "-- x\n" } });
    expect(marks(view)).toEqual([[14, 18]]);

    view.dispatch({ changes: { from: 15, to: 16, insert: "R" } });
    expect(marks(view)).toEqual([]);
    view.destroy();
  });

  it("is cleared on request", () => {
    const view = new EditorView({ state: state("SELECT nope") });
    showErrorMark(view, { at: 7, message: "x" });
    showErrorMark(view, null);
    expect(marks(view)).toEqual([]);
    view.destroy();
  });
});
