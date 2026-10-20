import { create } from "zustand";
import type { QueryScope, SavedQueryRef } from "../lib/queryTypes";
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

/** 一个 SQL 查询。id 一直不变(后端按它管这个标签页的专用连接),保存、换库都只改字段。 */
export interface QueryTab {
  kind: "query";
  id: string;
  title: string;
  connectionId: string;
  scope: QueryScope;
  /** 对应的保存的查询;新建、还没保存过的为 null。 */
  saved: SavedQueryRef | null;
  /** 打开时编辑器里的内容;之后的改动只在编辑器里,不回写到这里。 */
  initialSql: string;
}

export type Tab = ObjectsTab | TableDataTab | QueryTab;

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

const UNTITLED = /^查询 (\d+)$/;

/** 新查询的标题:「查询 n」,n 比开着的未命名查询都大。 */
export function nextQueryTitle(tabs: readonly Tab[]): string {
  const used = tabs.flatMap((t) => {
    const match = t.kind === "query" && !t.saved ? UNTITLED.exec(t.title) : null;
    return match ? [Number(match[1])] : [];
  });
  return `查询 ${Math.max(0, ...used) + 1}`;
}

export function queryTab(
  connectionId: string,
  scope: QueryScope,
  title: string,
  saved: SavedQueryRef | null = null,
  initialSql = "",
): QueryTab {
  return { kind: "query", id: `query:${crypto.randomUUID()}`, title, connectionId, scope, saved, initialSql };
}

export function sameSavedQuery(a: SavedQueryRef | null, b: SavedQueryRef): boolean {
  return (
    a !== null &&
    a.connectionId === b.connectionId &&
    a.database === b.database &&
    (a.schema ?? null) === (b.schema ?? null) &&
    a.name === b.name
  );
}

export type QueryTabPatch = Partial<Pick<QueryTab, "title" | "scope" | "saved">>;

export function updateQueryTab(state: TabsState, id: string, patch: QueryTabPatch): TabsState {
  return {
    ...state,
    tabs: state.tabs.map((t) => (t.id === id && t.kind === "query" ? { ...t, ...patch } : t)),
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

/** `keepQueries` 时留下查询标签页:SQL 还在,连接重新打开后照常能执行。 */
export function closeConnectionTabs(state: TabsState, connectionId: string, keepQueries = false): TabsState {
  return removeWhere(state, (t) => t.connectionId === connectionId && !(keepQueries && t.kind === "query"));
}

interface TabsStore extends TabsState {
  /** 表数据页未提交的修改行数,关闭前据此确认。 */
  pending: Readonly<Record<string, number>>;
  setPending(id: string, count: number): void;
  /** 有没保存的改动的查询标签页,关闭前同样要确认。 */
  unsaved: Readonly<Record<string, true>>;
  setUnsaved(id: string, unsaved: boolean): void;
  updateQuery(id: string, patch: QueryTabPatch): void;
  open(tab: Tab): void;
  close(id: string): void;
  closeConnection(connectionId: string, keepQueries?: boolean): void;
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
  updateQuery: (id, patch) => set((s) => updateQueryTab(s, id, patch)),
  open: (tab) => set((s) => openTab(s, tab)),
  close: (id) => set((s) => closeTab(s, id)),
  closeConnection: (connectionId, keepQueries) => set((s) => closeConnectionTabs(s, connectionId, keepQueries)),
  activate: (id) => set((s) => (s.tabs.some((t) => t.id === id) ? { activeId: id } : s)),
}));
