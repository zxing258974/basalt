// 保存的查询:对象树的「查询」分组和打开、保存、删除共用这里的缓存键。
import { useQuery } from "@tanstack/react-query";
import { getBackend } from "../lib/api";

export const savedQueryKeys = {
  all: ["savedQueries"] as const,
  list: (connectionId: string, database: string, schema: string | null) =>
    ["savedQueries", connectionId, database, schema ?? ""] as const,
};

export function useSavedQueries(connectionId: string, database: string, schema: string | null, enabled: boolean) {
  return useQuery({
    queryKey: savedQueryKeys.list(connectionId, database, schema),
    queryFn: async () => (await getBackend()).listSavedQueries(connectionId, database, schema),
    enabled,
  });
}
