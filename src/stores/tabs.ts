import { create } from "zustand";
import type { TableRef } from "../lib/types";

/** 某个库(PG 下是某个 schema)的对象列表。 */
export interface ObjectsTab {
  kind: "objects";
  id: string;
  title: string;
  connectionId: string;
  database: string;
  schema: string | null;
}

/** 一张表或视图的数据。 */
export interface TableDataTab {
  kind: "tableData";
  id: string;
  title: string;
  connectionId: string;
  table: TableRef;
}

/** 表结构设计器。`table` 为 null 是还没建的新表;建好、改名之后换成新表名的标签页。 */
export interface TableDesignTab {
  kind: "tableDesign";
  id: string;
  title: string;
  connectionId: string;
  database: string;
  schema: string | null;
  table: string | null;
}

export type Tab = ObjectsTab | TableDataTab | TableDesignTab;

export interface TabsState {
  tabs: Tab[];
  activeId: string | null;
}

export function objectsTab(connectionId: string, database: string, schema: string | null): ObjectsTab {
  return {
    kind: "objects",
    id: `objects:${connectionId}:${database}:${schema ?? ""}`,
    title: schema ? `${database}.${schema}` : database,
    connectionId,
    database,
    schema,
  };
}

export function tableDataTab(
  connectionId: string,
  database: string,
  schema: string | null,
  table: string,
): TableDataTab {
  // 各段编码后再拼,名字里带冒号也不会和别的表撞 id。
  const parts = [connectionId, database, schema ?? "", table].map(encodeURIComponent);
  return {
    kind: "tableData",
    id: `data:${parts.join(":")}`,
    title: table,
    connectionId,
    table: { database, schema, table },
  };
}

export function tableDesignTab(
  connectionId: string,
  database: string,
  schema: string | null,
  table: string,
): TableDesignTab {
  const parts = [connectionId, database, schema ?? "", table].map(encodeURIComponent);
  return { kind: "tableDesign", id: `design:${parts.join(":")}`, title: table, connectionId, database, schema, table };
}

/** 新建表:每次都是一个新标签页。 */
export function newTableTab(connectionId: string, database: string, schema: string | null): TableDesignTab {
  return {
    kind: "tableDesign",
    id: `design-new:${crypto.randomUUID()}`,
    title: "新表",
    connectionId,
    database,
    schema,
    table: null,
  };
}

/** 原地换成另一个标签页(建表、改名之后);新的已经开着就关掉旧的、激活新的。 */
export function replaceTab(state: TabsState, oldId: string, tab: Tab): TabsState {
  if (oldId === tab.id) return state;
  if (state.tabs.some((t) => t.id === tab.id)) {
    const closed = closeTab(state, oldId);
    return { ...closed, activeId: tab.id };
  }
  return {
    tabs: state.tabs.map((t) => (t.id === oldId ? tab : t)),
    activeId: state.activeId === oldId ? tab.id : state.activeId,
  };
}

export function openTab(state: TabsState, tab: Tab): TabsState {
  const exists = state.tabs.some((t) => t.id === tab.id);
  return {
    tabs: exists ? state.tabs : [...state.tabs, tab],
    activeId: tab.id,
  };
}

/** 关掉活动标签时优先激活右邻,没有右邻再激活左邻。 */
function nextActiveAfterRemoval(state: TabsState, remaining: Tab[]): string | null {
  if (remaining.some((t) => t.id === state.activeId)) return state.activeId;
  const activeIndex = state.tabs.findIndex((t) => t.id === state.activeId);
  const right = state.tabs.slice(activeIndex + 1).find((t) => remaining.includes(t));
  const left = state.tabs
    .slice(0, Math.max(activeIndex, 0))
    .reverse()
    .find((t) => remaining.includes(t));
  return (right ?? left)?.id ?? null;
}

function removeWhere(state: TabsState, predicate: (tab: Tab) => boolean): TabsState {
  const remaining = state.tabs.filter((t) => !predicate(t));
  if (remaining.length === state.tabs.length) return state;
  return { tabs: remaining, activeId: nextActiveAfterRemoval(state, remaining) };
}

export function closeTab(state: TabsState, id: string): TabsState {
  return removeWhere(state, (t) => t.id === id);
}

export function closeConnectionTabs(state: TabsState, connectionId: string): TabsState {
  return removeWhere(state, (t) => t.connectionId === connectionId);
}

interface TabsStore extends TabsState {
  /** 表数据页未提交的修改行数,关闭前据此确认。 */
  pending: Readonly<Record<string, number>>;
  setPending(id: string, count: number): void;
  /** 有没保存的改动的标签页(表结构设计器),关闭前同样要确认。 */
  unsaved: Readonly<Record<string, true>>;
  setUnsaved(id: string, unsaved: boolean): void;
  replace(oldId: string, tab: Tab): void;
  open(tab: Tab): void;
  close(id: string): void;
  closeConnection(connectionId: string): void;
  activate(id: string): void;
}

export const useTabs = create<TabsStore>((set) => ({
  tabs: [],
  activeId: null,
  pending: {},
  setPending: (id, count) =>
    set((s) => {
      const { [id]: _previous, ...rest } = s.pending;
      return { pending: count > 0 ? { ...rest, [id]: count } : rest };
    }),
  unsaved: {},
  setUnsaved: (id, unsaved) =>
    set((s) => {
      if (Boolean(s.unsaved[id]) === unsaved) return s;
      const { [id]: _previous, ...rest } = s.unsaved;
      return { unsaved: unsaved ? { ...rest, [id]: true } : rest };
    }),
  replace: (oldId, tab) => set((s) => replaceTab(s, oldId, tab)),
  open: (tab) => set((s) => openTab(s, tab)),
  close: (id) => set((s) => closeTab(s, id)),
  closeConnection: (connectionId) => set((s) => closeConnectionTabs(s, connectionId)),
  activate: (id) => set((s) => (s.tabs.some((t) => t.id === id) ? { activeId: id } : s)),
}));
