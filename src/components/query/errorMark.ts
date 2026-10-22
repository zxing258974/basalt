// 编辑器里标出语句错在哪:一个红色波浪线的标记,悬停看报错原文。
// 改到标记上就去掉(那段文字已经不是报错时的样子了),改别处时跟着挪。
import { StateEffect, StateField, type EditorState } from "@codemirror/state";
import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";

export interface ErrorMark {
  /** 文档里的位置(UTF-16 单元)。 */
  at: number;
  /** 这条语句在文档里的结尾:错在语句末尾(「语句不完整」)时标最后一个词,不标到下一条去。 */
  end?: number;
  message: string;
}

const setMark = StateEffect.define<{ from: number; to: number; message: string } | null>();

const WORD = /^[\p{L}\p{N}_$]$/u;
const SPACE = /^\s$/u;

/** `at` 处的一个字(emoji 这类占两个 UTF-16 单元的不拆开)。 */
function charAt(state: EditorState, at: number): string {
  const code = state.doc.sliceString(at, at + 2).codePointAt(0);
  return code === undefined ? "" : String.fromCodePoint(code);
}

/** `at` 前面的一个字。 */
function charBefore(state: EditorState, at: number): string {
  const pair = state.doc.sliceString(Math.max(0, at - 2), at);
  const last = pair.slice(-1);
  return pair.length === 2 && /[\uDC00-\uDFFF]/.test(last) && /[\uD800-\uDBFF]/.test(pair[0]!) ? pair : last;
}

/**
 * 标到出错处的那个词,落在标点上标一个字。落在空白上、语句末尾之后时往回找,
 * 标前面最后一个词(或一个字):「语句不完整」标在最后写的东西上。
 */
export function markRange(state: EditorState, at: number, end?: number): { from: number; to: number } | null {
  const limit = Math.min(end ?? state.doc.length, state.doc.length);
  const from = Math.min(Math.max(at, 0), limit);
  if (from >= limit || SPACE.test(charAt(state, from))) {
    let back = from;
    while (back > 0 && SPACE.test(charBefore(state, back))) back -= charBefore(state, back).length;
    if (back === 0) return null;
    let start = back - charBefore(state, back).length;
    if (WORD.test(charBefore(state, back))) {
      while (start > 0 && WORD.test(charBefore(state, start))) start -= charBefore(state, start).length;
    }
    return { from: start, to: back };
  }
  let to = from;
  while (to < limit && WORD.test(charAt(state, to))) to += charAt(state, to).length;
  if (to === from) to = from + charAt(state, from).length;
  return { from, to };
}

export const errorMarkField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    for (const effect of tr.effects) {
      if (!effect.is(setMark)) continue;
      if (!effect.value) return Decoration.none;
      const { from, to, message } = effect.value;
      return Decoration.set([
        Decoration.mark({ class: "cm-sql-error", attributes: { title: message } }).range(from, to),
      ]);
    }
    if (!tr.docChanged || marks.size === 0) return marks;
    let touched = false;
    marks.between(0, tr.startState.doc.length, (from, to) => {
      tr.changes.iterChangedRanges((changeFrom, changeTo) => {
        if (changeFrom <= to && changeTo >= from) touched = true;
      });
    });
    return touched ? Decoration.none : marks.map(tr.changes);
  },
  provide: (field) => EditorView.decorations.from(field),
});

/** 标出(并滚到)出错的地方;null 去掉标记。 */
export function showErrorMark(view: EditorView, mark: ErrorMark | null): void {
  const range = mark ? markRange(view.state, mark.at, mark.end) : null;
  if (!mark || !range) {
    view.dispatch({ effects: setMark.of(null) });
    return;
  }
  view.dispatch({
    effects: [setMark.of({ ...range, message: mark.message }), EditorView.scrollIntoView(range.from, { y: "nearest" })],
  });
}
