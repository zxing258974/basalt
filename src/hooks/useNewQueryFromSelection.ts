// 工具栏「新建查询」和 ⌘T:在对象树选中的连接(和它所在的库、schema)上开一个查询标签页;
// Redis 连接开的是命令行(选中的是某个库就在那个库)。
import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { connectionIdFromKey, locationFromKey, redisDbFromKey } from "../components/sidebar/treeKeys";
import type { ConnectionView } from "../lib/types";
import { useUi } from "../stores/ui";
import { connectionsQuery } from "./useCatalog";
import { useQueryTabs } from "./useQueryTabs";

export function useNewQueryFromSelection() {
  const queryClient = useQueryClient();
  const { openNewQuery, openRedisConsole } = useQueryTabs();
  return useCallback(async () => {
    const { selectedKey, notify } = useUi.getState();
    const id = connectionIdFromKey(selectedKey);
    const connections = queryClient.getQueryData<ConnectionView[]>(connectionsQuery.queryKey) ?? [];
    const connection = connections.find((c) => c.id === id);
    if (!connection) {
      notify("先在左边选一个连接，再新建查询。");
      return;
    }
    if (connection.driver === "redis") {
      await openRedisConsole(connection, redisDbFromKey(selectedKey)?.db);
      return;
    }
    const { database, schema } = locationFromKey(selectedKey);
    await openNewQuery(connection, { database: database ?? undefined, schema: schema ?? undefined });
  }, [openNewQuery, openRedisConsole, queryClient]);
}
