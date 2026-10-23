// 表操作的纯逻辑:新表的默认名字、哪些标签页属于这张表、改名后换成什么标签页、各家的说法、结果的说法。
import { tableDataTab, tableDesignTab, type Tab } from "../../stores/tabs";
import type {
  ClearMode,
  ClearTableOutcome,
  CopyTableOutcome,
  DriverKind,
  Maintenance,
  MaintenanceMessage,
  MaintenanceOutcome,
  ReferencingKey,
  TableRef,
  SqlDriverKind,
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
export const COPY_LIMITS: Record<SqlDriverKind, string> = {
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

export interface ClearChoice {
  mode: ClearMode;
  label: string;
  /** 执行的语句,写在选项右边。 */
  sql: string;
  detail: string;
}

const TRUNCATE_DETAIL: Record<SqlDriverKind, string> = {
  mysql:
    "直接重建成空表，很快；自增编号从 1 开始。会立即提交、不能回滚，不触发删除触发器；被别的表的外键引用时会失败。",
  mariadb:
    "直接重建成空表，很快；自增编号从 1 开始。会立即提交、不能回滚，不触发删除触发器；被别的表的外键引用时会失败。",
  postgres: "很快；标识列和 serial 的序列从头开始（RESTART IDENTITY），不触发删除触发器；被别的表的外键引用时会失败。",
  sqlite:
    "SQLite 没有 TRUNCATE：用不带条件的 DELETE 清空（别的表引用它的外键照样按规则连带），再把自增计数归零，新行从 1 开始。",
};

/** 清空表对话框里的两种做法。 */
export function clearChoices(driver: DriverKind): ClearChoice[] {
  return [
    {
      mode: "delete",
      label: "清空",
      sql: "DELETE FROM",
      detail:
        "逐行删除，出错会整体回滚，删除触发器照常触发，别的表引用它的外键按 ON DELETE 规则连带；自增编号保留，新行接着往后编。表大时较慢。",
    },
    {
      mode: "truncate",
      label: "截断",
      sql: driver === "sqlite" ? "DELETE FROM + 计数归零" : "TRUNCATE",
      detail: driver === "redis" ? "" : TRUNCATE_DETAIL[driver],
    },
  ];
}

export function clearedNotice(table: TableRef, mode: ClearMode, outcome: ClearTableOutcome): string {
  const verb = mode === "delete" ? "清空" : "截断";
  const rows = outcome.rows === null ? "" : `，删除了 ${outcome.rows.toLocaleString("zh-CN")} 行`;
  return `已${verb}表「${table.table}」${rows}。`;
}

const CHANGES = {
  cascade: "引用它的行会被删掉",
  setNull: "引用它的列会被置空",
  setDefault: "引用它的列会改回默认值",
} as const;

const unique = (items: string[]) => [...new Set(items)];

/** 外键所在的表:和这张表在同一个库(schema)里就只写表名。 */
function ownerName(table: TableRef, key: ReferencingKey): string {
  const same = key.table.database === table.database && (key.table.schema ?? null) === (table.schema ?? null);
  return same ? key.table.table : qualifiedName(key.table);
}

/** 会被清空连带改动的表(ON DELETE CASCADE / SET NULL / SET DEFAULT)。 */
function changedOwners(table: TableRef, keys: readonly ReferencingKey[]): string[] {
  return unique(keys.filter((k) => k.onDelete in CHANGES).map((k) => ownerName(table, k)));
}

/**
 * 清空会动到的表:这张表,加上按外键连带删除、置空、改默认值的那些。MySQL、PostgreSQL 的截断只要被引用就被拒绝,
 * 只动这张表;SQLite 的截断也是 DELETE,照样连带。开着的这些表的数据页要重读,里面没提交的修改要先问。
 */
export function clearTouches(
  table: TableRef,
  keys: readonly ReferencingKey[],
  mode: ClearMode,
  driver: DriverKind,
): TableRef[] {
  if (mode === "truncate" && driver !== "sqlite") return [table];
  return [table, ...keys.filter((k) => k.onDelete in CHANGES).map((k) => k.table)];
}

/**
 * 引用这张表的外键在确认框里的说法。`warning` 是会改动、会被拒绝的,用醒目的样式;`note` 是可能失败的。
 * MySQL、PostgreSQL 的截断只要有引用就被拒绝;清空(SQLite 的截断也是 DELETE)按 ON DELETE 规则连带。
 */
export function referenceNotes(
  table: TableRef,
  keys: readonly ReferencingKey[],
  mode: ClearMode,
  driver: DriverKind,
): { warning: string | null; note: string | null } {
  if (mode === "truncate" && driver !== "sqlite") {
    const owners = unique(keys.map((k) => ownerName(table, k)));
    return {
      warning: owners.length > 0 ? `被 ${owners.join("、")} 的外键引用：截断会被数据库拒绝。` : null,
      note: null,
    };
  }
  const changes = (Object.keys(CHANGES) as (keyof typeof CHANGES)[]).flatMap((rule) => {
    const owners = unique(keys.filter((k) => k.onDelete === rule).map((k) => ownerName(table, k)));
    return owners.length > 0 ? [`${owners.join("、")} 里${CHANGES[rule]}`] : [];
  });
  const blocking = unique(
    keys.filter((k) => k.onDelete === "restrict" || k.onDelete === "noAction").map((k) => ownerName(table, k)),
  );
  return {
    warning: changes.length > 0 ? `会按外键连带改动别的表：${changes.join("；")}。` : null,
    note: blocking.length > 0 ? `${blocking.join("、")} 也引用这张表：那边还有引用它的行时，会删不掉。` : null,
  };
}

/**
 * 清空、截断被外键挡住时,在数据库的原话后面补一句怎么办。MySQL、PostgreSQL 的截断只要有引用就被拒绝,
 * 可以改用清空(连带改动别的表时说清楚);清空和 SQLite 的截断(它也是 DELETE)被挡住,是别的表里还有
 * RESTRICT / NO ACTION 引用它的行,换哪种做法都一样,得先处理那些行。
 */
export function clearRefusal(
  message: string,
  mode: ClearMode,
  driver: DriverKind,
  table: TableRef,
  keys: readonly ReferencingKey[],
): string {
  if (!/foreign key/i.test(message)) return message;
  if (mode === "truncate" && driver !== "sqlite") {
    const owners = changedOwners(table, keys);
    return owners.length > 0
      ? `${message}\n被别的表的外键引用时不能截断。改用「清空」会按外键连带改动 ${owners.join("、")} 里的行。`
      : `${message}\n被别的表的外键引用时不能截断，可以改用「清空」。`;
  }
  const blocking = unique(
    keys.filter((k) => k.onDelete === "restrict" || k.onDelete === "noAction").map((k) => ownerName(table, k)),
  );
  const where = blocking.length > 0 ? `${blocking.join("、")} 里` : "别的表里";
  return `${message}\n${where}还有引用它的行，外键不让删：先删掉或改掉那些行再试。`;
}

/** 对象列表给的行数:MySQL、PostgreSQL 都是估算。 */
export function rowsEstimate(rows: number): string {
  return `约 ${rows.toLocaleString("zh-CN")} 行，估算`;
}

/** 改名之后,引用这张表的东西各家跟不跟着改。 */
export const RENAME_NOTES: Record<SqlDriverKind, string> = {
  mysql: "外键和触发器会跟着改；视图、存储过程里写的旧表名不会，要自己改。",
  mariadb: "外键和触发器会跟着改；视图、存储过程里写的旧表名不会，要自己改。",
  postgres: "视图、外键、触发器会跟着改；函数里写的旧表名不会，要自己改。",
  sqlite: "视图、触发器、外键里的表名会一起改。",
};

/** 转储文件开头会做的事,各家不一样。 */
export const DUMP_NOTES: Record<SqlDriverKind, string> = {
  mysql:
    "文件开头先删掉同名表（DROP TABLE IF EXISTS）再建表；表名不带库名，执行时进当前库。执行期间关掉外键检查，触发器在数据之后重建。",
  mariadb:
    "文件开头先删掉同名表（DROP TABLE IF EXISTS）再建表；表名不带库名，执行时进当前库。执行期间关掉外键检查，触发器在数据之后重建。",
  postgres:
    "文件开头先删掉同名表（DROP TABLE IF EXISTS）再建表，名字都带 schema；外键、触发器在数据之后加。表被别的表的外键或视图引用时，开头的 DROP 会失败。标识列一律写成 GENERATED BY DEFAULT，serial 序列的起始值、步长等选项不保留。",
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
export const MAINTENANCE: Record<SqlDriverKind, MaintenanceChoice[]> = {
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

/** 后端归为失败的消息(error 行,或者 Operation failed、Corrupt 这类 status):这次维护发现了问题。 */
export function maintenanceProblem(outcome: MaintenanceOutcome): MaintenanceMessage | null {
  return outcome.messages.find((m) => m.kind === "failure") ?? null;
}

/** 结果对话框的标题:发现问题时不说「完成」。 */
export function maintenanceTitle(label: string, outcome: MaintenanceOutcome): string {
  return maintenanceProblem(outcome) ? `${label}发现问题` : `${label}完成`;
}

/** 对话框已经先关掉时,维护的结果改用状态栏的一句话:有问题说问题,否则说数据库最后一条消息。 */
export function maintenanceNotice(table: TableRef, label: string, outcome: MaintenanceOutcome): string {
  const shown = maintenanceProblem(outcome) ?? outcome.messages.at(-1);
  const detail = shown ? `${levelLabel(shown.level)} ${shown.text}` : "数据库没有返回消息";
  return `「${table.table}」${maintenanceTitle(label, outcome)}：${detail}。`;
}

/** 用时:不到 1 秒写毫秒,否则写到小数点后一位。 */
export function formatDuration(ms: number): string {
  return ms < 1000 ? `${ms} 毫秒` : `${(ms / 1000).toFixed(1)} 秒`;
}

/** 按数据库类型取说明;Redis 没有表,也就没有这些说明。 */
export function driverNote(notes: Record<SqlDriverKind, string>, driver: DriverKind): string {
  return driver === "redis" ? "" : notes[driver];
}

export function maintenanceChoices(driver: DriverKind): MaintenanceChoice[] {
  return driver === "redis" ? [] : MAINTENANCE[driver];
}
