// 删除表、复制表的纯逻辑:新表的默认名字、哪些标签页属于这张表、结果的说法。
import type { Tab } from "../../stores/tabs";
import type { CopyTableOutcome, DriverKind, TableRef } from "../../lib/types";

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
