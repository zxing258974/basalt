// SQL 补全的数据来源:表名、库 / schema 名与对象树共用缓存,列名按需取、也缓存。
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef } from "react";
import { catalogQueries } from "../../hooks/useCatalog";
import type { QueryScope } from "../../lib/queryTypes";
import type { DriverKind } from "../../lib/types";
import type { CompletionProvider } from "./sqlCompletion";

/** 元数据不常变,补全时不必每敲一个字都去问数据库。 */
const STALE_MS = 60_000;

export function useCompletionProvider(
  connectionId: string,
  driver: DriverKind | null,
  scope: QueryScope,
): CompletionProvider {
  const queryClient = useQueryClient();
  // 提供者只建一次(编辑器里装的是它),库换了从 ref 读最新的。
  const current = useRef(scope);
  useEffect(() => {
    current.current = scope;
  }, [scope]);

  return useMemo<CompletionProvider>(() => {
    const fetch = <T>(options: { queryKey: readonly unknown[]; queryFn: () => Promise<T> }) =>
      queryClient.fetchQuery({ ...options, staleTime: STALE_MS });
    const isPostgres = driver === "postgres";
    /** 补全时的「库」和「schema」:PostgreSQL 的 namespace 是 schema,其余是库。 */
    const locate = (namespace: string | null) => {
      const { database, schema } = current.current;
      if (isPostgres) return { database: database ?? "postgres", schema: namespace ?? schema ?? "public" };
      return { database: namespace ?? database, schema: null };
    };
    return {
      async tables(namespace) {
        const { database, schema } = locate(namespace);
        if (!database) return [];
        const tables = await fetch(catalogQueries.tables(connectionId, database, schema));
        return tables.map((t) => t.name);
      },
      async columns(table, namespace) {
        const { database, schema } = locate(namespace);
        if (!database) return [];
        const columns = await fetch(catalogQueries.columns(connectionId, database, schema, table));
        return columns.map((c) => c.name);
      },
      async namespaces() {
        const database = current.current.database;
        if (!isPostgres) return fetch(catalogQueries.databases(connectionId));
        return database ? fetch(catalogQueries.schemas(connectionId, database)) : [];
      },
    };
  }, [connectionId, driver, queryClient]);
}
