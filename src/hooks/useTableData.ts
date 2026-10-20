// 表数据的读取。每一页按请求参数单独缓存;有未提交的修改时页面不会自动刷新。
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { getBackend } from "../lib/api";
import type { PageRequest, TableRef } from "../lib/types";

export const tableDataKeys = {
  connection: (connectionId: string) => ["tableData", connectionId] as const,
  table: (connectionId: string, table: TableRef) =>
    ["tableData", connectionId, table.database, table.schema ?? "", table.table] as const,
  page: (connectionId: string, table: TableRef, request: PageRequest) =>
    [...tableDataKeys.table(connectionId, table), request] as const,
};

export function useTablePage(connectionId: string, table: TableRef, request: PageRequest, enabled: boolean) {
  return useQuery({
    queryKey: tableDataKeys.page(connectionId, table, request),
    queryFn: async () => (await getBackend()).readTablePage(connectionId, table, request),
    enabled,
    // 换页、排序时先留着上一页,不闪成「加载中」。
    placeholderData: keepPreviousData,
    // 编辑按行号记在本页上,数据只在用户点刷新或提交后才重读。
    staleTime: Number.POSITIVE_INFINITY,
  });
}
