import type { TableInfo } from "../../lib/types";

export type KindFilter = "all" | "tables" | "views";

export interface TableFilter {
  kind: KindFilter;
  search: string;
}

export type SortKey = "name" | "rows" | "dataBytes" | "indexBytes" | "updatedAt";

export interface TableSort {
  key: SortKey;
  direction: "asc" | "desc";
}

/** 名称、类型以外、按数据有无决定是否显示的列,按显示顺序排列。 */
export const OPTIONAL_COLUMNS = ["rows", "dataBytes", "indexBytes", "engine", "updatedAt", "comment"] as const;
export type OptionalColumn = (typeof OPTIONAL_COLUMNS)[number];

const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });

export function filterTables(tables: readonly TableInfo[], { kind, search }: TableFilter): TableInfo[] {
  const needle = search.trim().toLocaleLowerCase();
  return tables.filter((table) => {
    if (kind === "tables" && table.kind !== "table") return false;
    if (kind === "views" && table.kind === "table") return false;
    if (!needle) return true;
    return (
      table.name.toLocaleLowerCase().includes(needle) || (table.comment ?? "").toLocaleLowerCase().includes(needle)
    );
  });
}

function compareValues(a: string | number | null, b: string | number | null): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return collator.compare(String(a), String(b));
}

/** 未知值(null)无论升序降序都排在最后。 */
export function sortTables(tables: readonly TableInfo[], { key, direction }: TableSort): TableInfo[] {
  const sign = direction === "asc" ? 1 : -1;
  return [...tables].sort((x, y) => {
    const a = x[key];
    const b = y[key];
    if (a === null && b === null) return collator.compare(x.name, y.name);
    if (a === null) return 1;
    if (b === null) return -1;
    return sign * compareValues(a, b);
  });
}

export function visibleColumns(tables: readonly TableInfo[]): OptionalColumn[] {
  return OPTIONAL_COLUMNS.filter((column) => tables.some((table) => table[column] !== null));
}
