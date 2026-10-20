import { create } from "zustand";

/** 某个库(PG 下是某个 schema)的对象列表。 */
export interface ObjectsTab {
  kind: "objects";
  id: string;
  title: string;
  connectionId: string;
  database: string;
  schema: string | null;
}

export type Tab = ObjectsTab;

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
  open(tab: Tab): void;
  close(id: string): void;
  closeConnection(connectionId: string): void;
  activate(id: string): void;
}

export const useTabs = create<TabsStore>((set) => ({
  tabs: [],
  activeId: null,
  open: (tab) => set((s) => openTab(s, tab)),
  close: (id) => set((s) => closeTab(s, id)),
  closeConnection: (connectionId) => set((s) => closeConnectionTabs(s, connectionId)),
  activate: (id) => set((s) => (s.tabs.some((t) => t.id === id) ? { activeId: id } : s)),
}));
