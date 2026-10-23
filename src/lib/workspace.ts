// 上次开着的标签页:关掉 Basalt 再打开,查询里写的 SQL、开着的表、对象列表和服务器监控都还在。
// 这里只管格式和换算,存取在 hooks/useWorkspace.ts,文件由后端写在配置目录的 workspace.json。
import type {
  MonitorTab,
  ObjectsTab,
  QueryTab,
  RedisKeysTab,
  Tab,
  TableDataTab,
  TableDesignTab,
  TabsState,
} from "../stores/tabs";

export const WORKSPACE_VERSION = 1;

/** 查询标签页另外记下编辑器里的 SQL,和上次保存(或打开)时的内容,用来恢复「没保存」的标记。 */
export interface SavedQueryTab extends Omit<QueryTab, "initialSql" | "baselineSql"> {
  sql: string;
  baselineSql: string;
}

/** 设计器只记已有的表:还没建出来的新表,设计内容本来就不记。 */
export type SavedTab =
  ObjectsTab | TableDataTab | (TableDesignTab & { table: string }) | SavedQueryTab | MonitorTab | RedisKeysTab;

export interface Workspace {
  version: typeof WORKSPACE_VERSION;
  activeId: string | null;
  tabs: SavedTab[];
}

/** 查询编辑器里现在的内容,和上次保存(或打开)时的内容。 */
export interface QueryDraft {
  sql: string;
  baseline: string;
}

function savedTab(tab: Tab, drafts: Readonly<Record<string, QueryDraft>>): SavedTab | null {
  switch (tab.kind) {
    case "query": {
      const { initialSql, baselineSql, ...rest } = tab;
      const draft = drafts[tab.id] ?? { sql: initialSql, baseline: baselineSql ?? initialSql };
      return { ...rest, sql: draft.sql, baselineSql: draft.baseline };
    }
    case "tableDesign":
      return tab.table === null ? null : { ...tab, table: tab.table };
    default:
      return tab;
  }
}

/** 现在开着的标签页 → 要记下来的内容。`waiting` 是还在等连接、没放回去的上次的标签页,也照原位置记着。 */
export function snapshot(
  state: TabsState,
  drafts: Readonly<Record<string, QueryDraft>>,
  waiting: readonly SavedTab[] = [],
  order: readonly string[] = [],
): Workspace {
  let tabs = state.tabs.flatMap((tab) => savedTab(tab, drafts) ?? []);
  for (const tab of waiting) {
    if (!tabs.some((t) => t.id === tab.id)) tabs = insertByOrder(tabs, tab, order);
  }
  const activeId = tabs.some((t) => t.id === state.activeId) ? state.activeId : null;
  return { version: WORKSPACE_VERSION, activeId, tabs };
}

/** 按上次的顺序放回:插在上次排在它前面、现在已经回来了的那个标签页后面。 */
export function insertByOrder<T extends { id: string }>(list: readonly T[], item: T, order: readonly string[]): T[] {
  const position = order.indexOf(item.id);
  const before = new Set(order.slice(0, Math.max(position, 0)));
  let index = 0;
  list.forEach((t, i) => {
    if (before.has(t.id)) index = i + 1;
  });
  if (position < 0) index = list.length;
  return [...list.slice(0, index), item, ...list.slice(index)];
}

// ---------- 读回来:格式不对的条目丢掉,不让一条坏记录拖垮启动 ----------

type Fields = Record<string, unknown>;

const isObject = (value: unknown): value is Fields => typeof value === "object" && value !== null;
const isString = (value: unknown): value is string => typeof value === "string";
const isNullableString = (value: unknown): value is string | null => value === null || isString(value);

function hasStrings(fields: Fields, keys: readonly string[]): boolean {
  return keys.every((key) => isString(fields[key]));
}

function isScope(value: unknown): boolean {
  return isObject(value) && isNullableString(value.database) && isNullableString(value.schema);
}

function isSavedRef(value: unknown): boolean {
  return (
    value === null ||
    (isObject(value) &&
      hasStrings(value, ["connectionId", "database", "name"]) &&
      (value.schema === undefined || isNullableString(value.schema)))
  );
}

function parseTab(value: unknown): SavedTab | null {
  if (!isObject(value) || !hasStrings(value, ["id", "title", "connectionId"])) return null;
  switch (value.kind) {
    case "query":
      return hasStrings(value, ["sql", "baselineSql"]) && isScope(value.scope) && isSavedRef(value.saved)
        ? (value as unknown as SavedQueryTab)
        : null;
    case "objects":
      return isString(value.database) && isNullableString(value.schema) ? (value as unknown as ObjectsTab) : null;
    case "tableData": {
      const table = value.table;
      return isObject(table) && hasStrings(table, ["database", "table"]) && isNullableString(table.schema)
        ? (value as unknown as TableDataTab)
        : null;
    }
    case "tableDesign":
      return hasStrings(value, ["database", "table"]) && isNullableString(value.schema)
        ? (value as unknown as SavedTab)
        : null;
    case "monitor":
      // id 由连接 id 定:对不上的话会和新开的同一个监控页重复。
      return value.id === `monitor:${value.connectionId as string}` ? (value as unknown as MonitorTab) : null;
    case "redisKeys": {
      // 同上,id 由连接和库号定。
      const db = value.db;
      const valid = typeof db === "number" && Number.isInteger(db) && db >= 0;
      return valid && value.id === `redis:${value.connectionId as string}:${db}`
        ? (value as unknown as RedisKeysTab)
        : null;
    }
    default:
      return null;
  }
}

/** 读 workspace.json 的内容;版本不认识或根本读不懂时返回 null。 */
export function parseWorkspace(text: string): Workspace | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObject(value) || value.version !== WORKSPACE_VERSION || !Array.isArray(value.tabs)) return null;
  const seen = new Set<string>();
  const tabs = value.tabs.flatMap((raw) => {
    const tab = parseTab(raw);
    if (!tab || seen.has(tab.id)) return [];
    seen.add(tab.id);
    return [tab];
  });
  const activeId = isString(value.activeId) && seen.has(value.activeId) ? value.activeId : null;
  return { version: WORKSPACE_VERSION, activeId, tabs };
}

/** 记下来的标签页 → 能直接放进标签栏的标签页。 */
export function restoredTab(tab: SavedTab): Tab {
  if (tab.kind !== "query") return tab;
  const { sql, baselineSql, ...rest } = tab;
  return { ...rest, initialSql: sql, baselineSql };
}

export interface RestorePlan {
  /** 马上放回去的:查询(SQL 不用连上库也能看、能改)。 */
  now: Tab[];
  /** 要等连接打开才能放回去的,按连接分组。 */
  waiting: Map<string, SavedTab[]>;
  /** 上次的顺序,等的标签页回来时照着插。 */
  order: string[];
  activeId: string | null;
}

/**
 * 查询标签页马上回来(连接删了也留着,里面的 SQL 不能悄悄没了);
 * 表数据、对象列表、设计器要等连接打开,连接已经不在了的就不恢复。
 */
export function planRestore(workspace: Workspace, connectionIds: ReadonlySet<string>): RestorePlan {
  const now: Tab[] = [];
  const waiting = new Map<string, SavedTab[]>();
  for (const tab of workspace.tabs) {
    if (tab.kind === "query") now.push(restoredTab(tab));
    else if (connectionIds.has(tab.connectionId))
      waiting.set(tab.connectionId, [...(waiting.get(tab.connectionId) ?? []), tab]);
  }
  return { now, waiting, order: workspace.tabs.map((t) => t.id), activeId: workspace.activeId };
}
