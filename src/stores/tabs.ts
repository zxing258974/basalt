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
  /** 上次保存(或打开)时的内容,判断有没有没保存的改动;不填就是 `initialSql`。从上次的记录恢复时才有。 */
  baselineSql?: string;
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

/** 一个连接的服务器监控:会话列表和指标。每个连接只开一个。 */
export interface MonitorTab {
  kind: "monitor";
  id: string;
  title: string;
  connectionId: string;
}

/** Redis 一个库的键:左边键列表,右边选中的键的值。每个连接的每个库只开一个。 */
export interface RedisKeysTab {
  kind: "redisKeys";
  id: string;
  title: string;
  connectionId: string;
  db: number;
}

/** Redis 命令行:每个标签页一条自己的连接,SELECT、MULTI 只对它生效。一个连接可以开几个。 */
export interface RedisConsoleTab {
  kind: "redisConsole";
  id: string;
  title: string;
  connectionId: string;
  /** 打开时在哪个库;之后 SELECT 换了库只在界面上跟着变。 */
  db: number;
}

export type Tab = ObjectsTab | TableDataTab | QueryTab | TableDesignTab | MonitorTab | RedisKeysTab | RedisConsoleTab;

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

export function monitorTab(connectionId: string): MonitorTab {
  return { kind: "monitor", id: `monitor:${connectionId}`, title: "服务器监控", connectionId };
}

export function redisKeysTab(connectionId: string, db: number): RedisKeysTab {
  return { kind: "redisKeys", id: `redis:${connectionId}:${db}`, title: `db${db}`, connectionId, db };
}

const CONSOLE_TITLE = /^命令行(?: (\d+))?$/;

/** 新开一个命令行。标题「命令行」「命令行 2」……,编号比这个连接开着的都大。 */
export function redisConsoleTab(connectionId: string, db: number, tabs: readonly Tab[]): RedisConsoleTab {
  const used = tabs.flatMap((t) => {
    const match = t.kind === "redisConsole" && t.connectionId === connectionId ? CONSOLE_TITLE.exec(t.title) : null;
    return match ? [Number(match[1] ?? 1)] : [];
  });
  const n = Math.max(0, ...used) + 1;
  return {
    kind: "redisConsole",
    id: `redis-console:${connectionId}:${crypto.randomUUID()}`,
    title: n === 1 ? "命令行" : `命令行 ${n}`,
    connectionId,
    db,
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

/** `keepQueries` 时留下查询标签页:SQL 还在,连接重新打开后照常能执行。 */
export function closeConnectionTabs(state: TabsState, connectionId: string, keepQueries = false): TabsState {
  return removeWhere(state, (t) => t.connectionId === connectionId && !(keepQueries && t.kind === "query"));
}

interface TabsStore extends TabsState {
  /** 表数据页未提交的修改行数,关闭前据此确认。 */
  pending: Readonly<Record<string, number>>;
  setPending(id: string, count: number): void;
  /** 有没保存的改动的标签页(查询、表结构设计器),关闭前同样要确认。 */
  unsaved: Readonly<Record<string, true>>;
  setUnsaved(id: string, unsaved: boolean): void;
  updateQuery(id: string, patch: QueryTabPatch): void;
  replace(oldId: string, tab: Tab): void;
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
  replace: (oldId, tab) => set((s) => replaceTab(s, oldId, tab)),
  open: (tab) => set((s) => openTab(s, tab)),
  close: (id) => set((s) => closeTab(s, id)),
  closeConnection: (connectionId, keepQueries) => set((s) => closeConnectionTabs(s, connectionId, keepQueries)),
  activate: (id) => set((s) => (s.tabs.some((t) => t.id === id) ? { activeId: id } : s)),
}));
