// 查询标签页测试的共用工具:jsdom 里驱动 CodeMirror、给虚拟滚动一个尺寸。
import { EditorView } from "@codemirror/view";
import { screen } from "@testing-library/react";
import { afterAll, beforeAll } from "vitest";

/**
 * jsdom 不做布局:CodeMirror 量光标位置要 Range 的矩形,虚拟滚动的表格要容器尺寸。
 * 在 describe 外面调用一次,装好桩、测完恢复。
 */
export function installLayoutStubs(): void {
  const sizes = { clientHeight: 600, clientWidth: 1000 } as const;
  const originals = Object.keys(sizes).map((key) => Object.getOwnPropertyDescriptor(Element.prototype, key));
  const rects = {
    getClientRects: Range.prototype.getClientRects,
    getBoundingClientRect: Range.prototype.getBoundingClientRect,
  };
  beforeAll(() => {
    for (const [key, value] of Object.entries(sizes)) {
      Object.defineProperty(Element.prototype, key, { configurable: true, get: () => value });
    }
    Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
    Range.prototype.getBoundingClientRect = () => new DOMRect();
  });
  afterAll(() => {
    Object.keys(sizes).forEach((key, i) => Object.defineProperty(Element.prototype, key, originals[i]!));
    Range.prototype.getClientRects = rects.getClientRects;
    Range.prototype.getBoundingClientRect = rects.getBoundingClientRect;
  });
}

/** 标签页标题为 title 的 SQL 编辑器。 */
export async function findEditor(title: string): Promise<EditorView> {
  const content = await screen.findByRole("textbox", { name: `${title} 的 SQL` });
  const view = EditorView.findFromDOM(content);
  if (!view) throw new Error(`找不到「${title}」的编辑器`);
  return view;
}

/** 换掉编辑器里的全部内容,光标放在 cursor(默认末尾),或选中 [from, to)。 */
export function setSql(view: EditorView, text: string, selection?: number | { from: number; to: number }): void {
  const range =
    selection === undefined
      ? { anchor: text.length }
      : typeof selection === "number"
        ? { anchor: selection }
        : { anchor: selection.from, head: selection.to };
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, selection: range });
}
