// 表结构的读取。设计器里有没保存的修改,结构只在保存后或用户刷新时才重读。
import { useQuery } from "@tanstack/react-query";
import { getBackend } from "../lib/api";
import type { TableRef } from "../lib/types";

export const designKeys = {
  connection: (connectionId: string) => ["tableDesign", connectionId] as const,
  table: (connectionId: string, table: TableRef) =>
    ["tableDesign", connectionId, table.database, table.schema ?? "", table.table] as const,
};

export function designQuery(connectionId: string, table: TableRef) {
  return {
    queryKey: designKeys.table(connectionId, table),
    queryFn: async () => (await getBackend()).readTableDesign(connectionId, table),
  };
}

/** `table` 为 null(新表)时不读。 */
export function useTableDesign(connectionId: string, table: TableRef | null, enabled: boolean) {
  return useQuery({
    ...designQuery(connectionId, table ?? { database: "", schema: null, table: "" }),
    enabled: enabled && table !== null,
    staleTime: Number.POSITIVE_INFINITY,
  });
}
