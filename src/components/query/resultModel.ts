// 查询结果交给 M2 的 DataGrid 显示:把已经取到前端的行包成 TablePage。排序在后端做(见 useResultRows)。
import type { ResultColumn, ResultSet, ResultSort } from "../../lib/queryTypes";
import type { CellValue, ColumnInfo, SortSpec, TablePage } from "../../lib/types";

/** 只读结果的列:按文本显示。 */
export function readOnlyColumn(column: ResultColumn): ColumnInfo {
  return {
    name: column.name,
    dataType: column.dataType,
    isPrimaryKey: false,
    nullable: true,
    hasDefault: false,
    isGenerated: false,
    valueType: { kind: "text" },
  };
}

/** `columns`:能改时用对上表结构的列,只读时为 null。 */
export function resultPage(
  set: ResultSet,
  rows: CellValue[][],
  columns: ColumnInfo[] | null,
  readOnlyReason: string | null,
): TablePage {
  return {
    columns: columns ?? set.columns.map(readOnlyColumn),
    rows,
    hasMore: false,
    readOnlyReason,
    sql: "",
  };
}

/**
 * DataGrid 按列名记排序。结果集里可能有同名列(SELECT a.id, b.id),排序时要按序号区分,
 * 所以 SortSpec.column 里放的是「序号:列名」。
 */
export const sortKey = (index: number, name: string) => `${index}:${name}`;

/** 交给后端的排序:按列序号。 */
export function toResultSort(sort: SortSpec | null): ResultSort | null {
  if (!sort) return null;
  const column = Number(sort.column.split(":")[0]);
  return Number.isInteger(column) && column >= 0 ? { column, descending: sort.descending } : null;
}

/** 下一批取多少行:越往后一批越大,翻到很靠后时不用来回太多次;后端一次最多给 5000 行。 */
export function batchSize(loaded: number): number {
  return Math.min(5000, Math.max(1000, Math.floor(loaded / 4)));
}
