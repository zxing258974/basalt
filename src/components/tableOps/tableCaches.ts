// 一张表在内存里的缓存:数据页、表结构、信息面板、补全用的列名、引用它的外键。
import type { QueryClient } from "@tanstack/react-query";
import { catalogKeys } from "../../hooks/useCatalog";
import { tableDataKeys } from "../../hooks/useTableData";
import { designKeys } from "../../hooks/useTableDesign";
import type { TableRef } from "../../lib/types";

/**
 * 数据页、表结构的缓存不会自己过期。表删了、改了名、新建出同名的表(复制、改名改成它)时要清掉,
 * 否则那个名字会先显示旧表的内容,在数据页里编辑提交还会按旧表的主键去改新表的行。
 */
export function forgetTable(queryClient: QueryClient, connectionId: string, table: TableRef): void {
  const keys = [
    tableDataKeys.table(connectionId, table),
    designKeys.table(connectionId, table),
    catalogKeys.details(connectionId, table.database, table.schema, table.table),
    catalogKeys.columns(connectionId, table.database, table.schema, table.table),
    catalogKeys.references(connectionId, table.database, table.schema, table.table),
  ];
  for (const queryKey of keys) queryClient.removeQueries({ queryKey });
}
