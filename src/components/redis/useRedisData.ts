// Redis 键浏览的读取(TanStack Query)。查询键都挂在 catalog 下:刷新、关闭连接时随连接一起失效。
import { useInfiniteQuery, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import type { RedisKind } from "../../lib/redisTypes";

/** 扫描一批凑够这么多个键就先显示。 */
export const SCAN_BATCH = 500;

export const redisKeys = {
  db: (id: string, db: number) => [...catalogKeys.connection(id), "redis", db] as const,
  scan: (id: string, db: number, pattern: string, kind: RedisKind | null) =>
    [...catalogKeys.connection(id), "redis", db, "scan", pattern, kind ?? ""] as const,
  key: (id: string, db: number, key: string) => [...catalogKeys.connection(id), "redis", db, "key", key] as const,
};

/** 按模式和类型扫描:第一批自动取,后面的点「继续扫描」再取。 */
export function useKeyScan(id: string, db: number, pattern: string, kind: RedisKind | null) {
  return useInfiniteQuery({
    queryKey: redisKeys.scan(id, db, pattern, kind),
    initialPageParam: "0",
    queryFn: async ({ pageParam }) =>
      (await getBackend()).redisScanKeys(id, db, { pattern, kind, cursor: pageParam, count: SCAN_BATCH }),
    getNextPageParam: (last) => (last.cursor === "0" ? undefined : last.cursor),
  });
}

export function useKeyDetail(id: string, db: number, key: string | null) {
  return useQuery({
    queryKey: redisKeys.key(id, db, key ?? ""),
    queryFn: async () => (await getBackend()).redisReadKey(id, db, key!),
    enabled: key !== null,
    retry: false,
  });
}

/**
 * 改过键之后:列表、这个库的键数重读;改的是 `key` 就重读它,
 * 新建、改名、删除这类换了名字的,这个库里读过的键全部作废(同名的键可能刚删了又建)。
 */
export async function refreshAfterChange(client: QueryClient, id: string, db: number, key?: string): Promise<void> {
  await Promise.all([
    client.invalidateQueries({ queryKey: [...redisKeys.db(id, db), "scan"] }),
    client.invalidateQueries({ queryKey: catalogKeys.redisKeyspace(id) }),
    client.invalidateQueries({
      queryKey: key === undefined ? [...redisKeys.db(id, db), "key"] : redisKeys.key(id, db, key),
    }),
  ]);
}

export function useRefreshAfterChange(id: string, db: number) {
  const client = useQueryClient();
  return useCallback((key?: string) => refreshAfterChange(client, id, db, key), [client, id, db]);
}
