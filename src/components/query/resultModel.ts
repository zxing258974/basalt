// 查询结果交给 M2 的 DataGrid 显示:把结果集包成只读的 TablePage,排序在前端做。
import type { ResultSet } from "../../lib/queryTypes";
import type { CellValue, ColumnInfo, SortSpec, TablePage } from "../../lib/types";

export const RESULT_READ_ONLY = "查询结果只读。要改数据，请打开表数据。";

function column(name: string, dataType: string): ColumnInfo {
  return {
    name,
    dataType,
    isPrimaryKey: false,
    nullable: true,
    hasDefault: false,
    isGenerated: false,
    valueType: { kind: "text" },
  };
}

const NUMERIC = new Set<CellValue["kind"]>(["int", "uint", "float", "decimal"]);

/** NULL 排最前;数字按数值比,其余按文本比(中文按拼音)。 */
export function compareCells(a: CellValue, b: CellValue): number {
  if (a.kind === "null" || b.kind === "null") return a.kind === b.kind ? 0 : a.kind === "null" ? -1 : 1;
  if (NUMERIC.has(a.kind) && NUMERIC.has(b.kind) && "value" in a && "value" in b) {
    const diff = Number(a.value) - Number(b.value);
    if (diff !== 0 && Number.isFinite(diff)) return Math.sign(diff);
  }
  const text = (cell: CellValue) => ("value" in cell ? String(typeof cell.value === "object" ? "" : cell.value) : "");
  return text(a).localeCompare(text(b), "zh-CN", { numeric: true });
}

/** 按列序号排序(同名列也能分开),稳定排序。 */
export function sortRows(rows: readonly CellValue[][], index: number, descending: boolean): CellValue[][] {
  return rows
    .map((row, at) => ({ row, at }))
    .sort((x, y) => compareCells(x.row[index]!, y.row[index]!) * (descending ? -1 : 1) || x.at - y.at)
    .map(({ row }) => row);
}

/**
 * DataGrid 按列名记排序。结果集里可能有同名列(SELECT a.id, b.id),排序时要按序号区分,
 * 所以 SortSpec.column 里放的是「序号:列名」。
 */
export const sortKey = (index: number, name: string) => `${index}:${name}`;

export function resultPage(set: ResultSet, sort: SortSpec | null): TablePage {
  const index = sort ? Number(sort.column.split(":")[0]) : -1;
  const rows = index >= 0 && index < set.columns.length ? sortRows(set.rows, index, sort!.descending) : set.rows;
  return {
    columns: set.columns.map((c) => column(c.name, c.dataType)),
    rows,
    hasMore: false,
    readOnlyReason: RESULT_READ_ONLY,
    sql: "",
  };
}
