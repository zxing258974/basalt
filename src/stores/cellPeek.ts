// 右侧信息面板「值」页看的那一格:表格(表数据页、查询结果)选中的格子一变就报上来,
// 面板只显示当前标签页报的那一格。同一个标签页里换了结果集,由新挂上的表格接着报。
import { create } from "zustand";

/** 一格的完整内容。 */
export type PeekValue =
  | { kind: "null" }
  | { kind: "text"; text: string }
  | { kind: "bytes"; hex: string }
  | { kind: "unsupported"; typeName: string }
  /** 新增行里还没填的格子:提交后会是什么。 */
  | { kind: "unset"; hint: string };

export interface CellPeek {
  /** 出处:表名,或「查询 1 的结果 2」。 */
  source: string;
  column: string;
  dataType: string;
  /** 表格行号槽里的序号;新增行为 null。 */
  rowNumber: number | null;
  value: PeekValue;
  /** 显示的是还没提交的新值。 */
  edited: boolean;
  /** 这一行标了删除,提交后就没了。 */
  deleted: boolean;
}

function sameValue(a: PeekValue, b: PeekValue): boolean {
  switch (a.kind) {
    case "null":
      return b.kind === "null";
    case "text":
      return b.kind === "text" && a.text === b.text;
    case "bytes":
      return b.kind === "bytes" && a.hex === b.hex;
    case "unsupported":
      return b.kind === "unsupported" && a.typeName === b.typeName;
    case "unset":
      return b.kind === "unset" && a.hint === b.hint;
  }
}

/** 看到的是不是同一样东西。表格重新渲染时常常算出内容一样的新对象,一样就不用报。 */
export function samePeek(a: CellPeek | null, b: CellPeek | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.source === b.source &&
    a.column === b.column &&
    a.dataType === b.dataType &&
    a.rowNumber === b.rowNumber &&
    a.edited === b.edited &&
    a.deleted === b.deleted &&
    sameValue(a.value, b.value)
  );
}

interface CellPeekStore {
  tabId: string | null;
  /** 哪个表格报的:它卸载时只清自己报的,不清别的表格刚报上来的。 */
  owner: string | null;
  peek: CellPeek | null;
  publish(tabId: string, owner: string, peek: CellPeek | null): void;
  release(owner: string): void;
}

export const useCellPeek = create<CellPeekStore>((set) => ({
  tabId: null,
  owner: null,
  peek: null,
  publish: (tabId, owner, peek) =>
    set((s) => (s.tabId === tabId && s.owner === owner && samePeek(s.peek, peek) ? s : { tabId, owner, peek })),
  release: (owner) => set((s) => (s.owner === owner ? { tabId: null, owner: null, peek: null } : s)),
}));
