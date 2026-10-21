// 表操作的纯逻辑:新表的默认名字、哪些标签页属于这张表、改名后换成什么标签页、各家的说法、结果的说法。
import { tableDataTab, tableDesignTab, type Tab, type TabsState } from "../../stores/tabs";
import type {
  ClearMode,
  ClearTableOutcome,
  CopyTableOutcome,
  DriverKind,
  Maintenance,
  TableRef,
} from "../../lib/types";

/** 「名字_copy1」,已经有了就往后数。 */
export function copyName(table: string, existing: readonly string[]): string {
  const taken = new Set(existing);
  for (let n = 1; ; n += 1) {
    const name = `${table}_copy${n}`;
    if (!taken.has(name)) return name;
  }
}

/** 开着这张表的数据页或设计器的标签页:删表前要先关掉(有没保存的修改就先问)。 */
export function tabsOfTable(tab: Tab, connectionId: string, table: TableRef): boolean {
  if (tab.connectionId !== connectionId) return false;
  if (tab.kind === "tableData") {
    return (
      tab.table.database === table.database &&
      (tab.table.schema ?? null) === (table.schema ?? null) &&
      tab.table.table === table.table
    );
  }
  if (tab.kind === "tableDesign") {
    return tab.database === table.database && tab.schema === (table.schema ?? null) && tab.table === table.table;
  }
  return false;
}

export function copiedNotice(outcome: CopyTableOutcome): string {
  const rows = outcome.rows === null ? "只复制了结构" : `复制了 ${outcome.rows.toLocaleString("zh-CN")} 行`;
  return `已复制为「${outcome.table.table}」，${rows}。`;
}

/** 如 shop.customers、FOLIO.public.users。 */
export function qualifiedName(table: TableRef): string {
  return [table.database, table.schema, table.table].filter(Boolean).join(".");
}

/** 各家复制表时带不过去的东西。 */
export const COPY_LIMITS: Record<DriverKind, string> = {
  mysql: "按 CREATE TABLE … LIKE 复制：字段、索引、表选项都一样；外键和触发器不会复制。",
  mariadb: "按 CREATE TABLE … LIKE 复制：字段、索引、表选项都一样；外键和触发器不会复制。",
  postgres:
    "按 LIKE … INCLUDING ALL 复制：默认值、约束、索引、标识列、注释都一样；外键和触发器不会复制，serial 列仍用原表的序列。",
  sqlite: "照原表的建表语句复制：约束、索引都一样；触发器不会复制。",
};

/** 改名之后,这张表的数据页、设计器换成新名字的标签页;别的标签页返回 null(不动)。 */
export function renamedTab(tab: Tab, connectionId: string, table: TableRef, name: string): Tab | null {
  if (!tabsOfTable(tab, connectionId, table)) return null;
  return tab.kind === "tableData"
    ? tableDataTab(connectionId, table.database, table.schema, name)
    : tableDesignTab(connectionId, table.database, table.schema ?? null, name);
}

type TabsSnapshot = Pick<TabsState, "tabs"> & {
  pending: Readonly<Record<string, number>>;
  unsaved: Readonly<Record<string, true>>;
};

/** 这张表开着的标签页里还没提交的表格修改行数、没保存的设计器个数。 */
export function pendingWork(
  state: TabsSnapshot,
  connectionId: string,
  table: TableRef,
): { rows: number; designs: number } {
  const mine = state.tabs.filter((tab) => tabsOfTable(tab, connectionId, table));
  return {
    rows: mine.reduce((sum, tab) => sum + (state.pending[tab.id] ?? 0), 0),
    designs: mine.filter((tab) => tab.kind === "tableDesign" && state.unsaved[tab.id]).length,
  };
}

/** 操作会丢掉的修改,写进确认框;没有就是 null。`action` 如「清空」「改名」。 */
export function pendingWarning(work: { rows: number; designs: number }, action: string): string | null {
  const parts = [
    work.rows > 0 ? ` ${work.rows} 行修改没有提交` : null,
    work.designs > 0 ? "设计器里的结构修改没有保存" : null,
  ].filter((part) => part !== null);
  return parts.length > 0 ? `这张表还有${parts.join("、")}，${action}后就没有了。` : null;
}

export interface ClearChoice {
  mode: ClearMode;
  label: string;
  /** 执行的语句,写在选项右边。 */
  sql: string;
  detail: string;
}

const TRUNCATE_DETAIL: Record<DriverKind, string> = {
  mysql:
    "直接重建成空表，很快；自增编号从 1 开始。会立即提交、不能回滚，不触发删除触发器；被别的表的外键引用时会失败。",
  mariadb:
    "直接重建成空表，很快；自增编号从 1 开始。会立即提交、不能回滚，不触发删除触发器；被别的表的外键引用时会失败。",
  postgres: "很快；标识列和 serial 的序列从头开始（RESTART IDENTITY），不触发删除触发器；被别的表的外键引用时会失败。",
  sqlite: "SQLite 没有 TRUNCATE：用不带条件的 DELETE 清空，再把自增计数归零，新行从 1 开始。",
};

/** 清空表对话框里的两种做法。 */
export function clearChoices(driver: DriverKind): ClearChoice[] {
  return [
    {
      mode: "delete",
      label: "清空",
      sql: "DELETE FROM",
      detail: "逐行删除，出错会整体回滚，删除触发器照常触发；自增编号保留，新行接着往后编。表大时较慢。",
    },
    {
      mode: "truncate",
      label: "截断",
      sql: driver === "sqlite" ? "DELETE FROM + 计数归零" : "TRUNCATE",
      detail: TRUNCATE_DETAIL[driver],
    },
  ];
}

export function clearedNotice(table: TableRef, mode: ClearMode, outcome: ClearTableOutcome): string {
  const verb = mode === "delete" ? "清空" : "截断";
  const rows = outcome.rows === null ? "" : `，删除了 ${outcome.rows.toLocaleString("zh-CN")} 行`;
  return `已${verb}表「${table.table}」${rows}。`;
}

/** 截断被外键挡住时,在数据库的原话后面补一句怎么办。 */
export function truncateRefusal(message: string): string {
  return /foreign key/i.test(message) ? `${message}\n被别的表的外键引用时不能截断，可以改用「清空」。` : message;
}

/** 改名之后,引用这张表的东西各家跟不跟着改。 */
export const RENAME_NOTES: Record<DriverKind, string> = {
  mysql: "外键和触发器会跟着改；视图、存储过程里写的旧表名不会，要自己改。",
  mariadb: "外键和触发器会跟着改；视图、存储过程里写的旧表名不会，要自己改。",
  postgres: "视图、外键、触发器会跟着改；函数里写的旧表名不会，要自己改。",
  sqlite: "视图、触发器、外键里的表名会一起改。",
};

/** 转储文件开头会做的事,各家不一样。 */
export const DUMP_NOTES: Record<DriverKind, string> = {
  mysql: "文件开头先删掉同名表（DROP TABLE IF EXISTS）再建表；表名不带库名，执行时进当前库。执行期间关掉外键检查。",
  mariadb: "文件开头先删掉同名表（DROP TABLE IF EXISTS）再建表；表名不带库名，执行时进当前库。执行期间关掉外键检查。",
  postgres:
    "文件开头先删掉同名表（DROP TABLE IF EXISTS）再建表，表名带 schema；建表语句按表结构生成，serial 的序列一起重建。",
  sqlite: "文件开头先删掉同名表（DROP TABLE IF EXISTS）再建表；索引跟在建表后面，触发器放在数据之后。",
};

export interface MaintenanceChoice {
  operation: Maintenance;
  label: string;
  sql: string;
  detail: string;
}

const MYSQL_MAINTENANCE: MaintenanceChoice[] = [
  {
    operation: "analyze",
    label: "分析",
    sql: "ANALYZE TABLE",
    detail: "重新统计索引里值的分布，帮优化器选对执行计划。很快。",
  },
  { operation: "check", label: "检查", sql: "CHECK TABLE", detail: "检查表和索引有没有错误。" },
  {
    operation: "optimize",
    label: "优化",
    sql: "OPTIMIZE TABLE",
    detail: "整理碎片、回收删掉的行占的空间。InnoDB 会重建整张表，大表要等一会儿。",
  },
  {
    operation: "repair",
    label: "修复",
    sql: "REPAIR TABLE",
    detail: "修复损坏的 MyISAM、Aria 表。InnoDB 不支持，数据库会提示不支持。",
  },
];

/** 各家能做的维护,按菜单里的顺序。 */
export const MAINTENANCE: Record<DriverKind, MaintenanceChoice[]> = {
  mysql: MYSQL_MAINTENANCE,
  mariadb: MYSQL_MAINTENANCE,
  postgres: [
    {
      operation: "vacuum",
      label: "清理",
      sql: "VACUUM",
      detail: "回收删除、更新留下的死行，空间留给以后的写入；一般不会让文件变小。",
    },
    {
      operation: "analyze",
      label: "分析",
      sql: "ANALYZE",
      detail: "重新统计数据的分布，帮优化器选对执行计划。",
    },
    {
      operation: "reindex",
      label: "重建索引",
      sql: "REINDEX TABLE",
      detail: "重建这张表的全部索引，去掉索引膨胀。期间会挡住对这张表的写入。",
    },
  ],
  sqlite: [
    {
      operation: "analyze",
      label: "分析",
      sql: "ANALYZE",
      detail: "统计索引的分布，写进 sqlite_stat1，帮查询规划器选索引。",
    },
    {
      operation: "vacuum",
      label: "整理库文件",
      sql: "VACUUM",
      detail: "重写整个数据库文件、回收空闲页。作用于整个库，不只这张表；库大时要等一会儿。",
    },
  ],
};

const LEVELS: Record<string, string> = { status: "状态", error: "错误", info: "信息", note: "提示", warning: "警告" };

/** MySQL 消息的级别,翻成中文;认不出的原样显示。 */
export function levelLabel(level: string): string {
  return LEVELS[level] ?? level;
}

/** 用时:不到 1 秒写毫秒,否则写到小数点后一位。 */
export function formatDuration(ms: number): string {
  return ms < 1000 ? `${ms} 毫秒` : `${(ms / 1000).toFixed(1)} 秒`;
}
