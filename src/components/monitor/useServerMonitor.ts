import { useQuery, useQueryClient } from "@tanstack/react-query";
import { getBackend } from "../../lib/api";
import { EMPTY_HISTORY, recordSnapshot, type MonitorHistory } from "./monitorModel";

export const monitorKey = (connectionId: string) => ["monitor", connectionId] as const;

interface MonitorOptions {
  connectionId: string;
  /** 连接开着才去取。 */
  enabled: boolean;
  /** 毫秒;null 是暂停。标签页切走了也传 null,不在背后一直查服务器。 */
  intervalMs: number | null;
}

/**
 * 隔一会儿取一份快照,攒成走势。走势就存在这条查询的缓存里:每次取到新的快照接在上一份后面,
 * 取失败时留着原来的。关掉标签页缓存就丢掉(gcTime 0),下次打开从头攒。
 */
export function useServerMonitor({ connectionId, enabled, intervalMs }: MonitorOptions) {
  const queryClient = useQueryClient();
  const key = monitorKey(connectionId);
  const query = useQuery({
    queryKey: key,
    queryFn: async (): Promise<MonitorHistory> => {
      const snapshot = await (await getBackend()).serverSnapshot(connectionId);
      return recordSnapshot(queryClient.getQueryData<MonitorHistory>(key) ?? EMPTY_HISTORY, snapshot);
    },
    enabled,
    refetchInterval: intervalMs ?? false,
    gcTime: 0,
    retry: false,
  });
  return { query, history: query.data ?? EMPTY_HISTORY };
}
