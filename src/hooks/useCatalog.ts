// 连接与元数据的读取(TanStack Query)。查询键集中在这里,方便统一失效。
import { skipToken, useQuery } from "@tanstack/react-query";
import type { ServerInfo, TableRef } from "../lib/types";
import { getBackend } from "../lib/api";

export const catalogKeys = {
  connections: ["connections"] as const,
  connection: (id: string) => ["catalog", id] as const,
  server: (id: string) => ["catalog", id, "server"] as const,
  databases: (id: string) => ["catalog", id, "databases"] as const,
  schemas: (id: string, database: string) => ["catalog", id, "schemas", database] as const,
  tables: (id: string, database: string, schema: string | null) =>
    ["catalog", id, "tables", database, schema ?? ""] as const,
  /** SQL 助手:一个库(schema)的表结构概要。 */
  outline: (id: string, database: string, schema: string | null) =>
    ["catalog", id, "outline", database, schema ?? ""] as const,
  /** 信息面板:一张表的属性和建表语句。 */
  details: (id: string, database: string, schema: string | null, table: string) =>
    ["catalog", id, "details", database, schema ?? "", table] as const,
  /** 补全用的列名,只在用到时才取。 */
  columns: (id: string, database: string, schema: string | null, table: string) =>
    ["catalog", id, "columns", database, schema ?? "", table] as const,
};

/** 对象树和 SQL 补全共用的查询定义:同一个 key,同一份缓存。 */
export const catalogQueries = {
  databases: (id: string) => ({
    queryKey: catalogKeys.databases(id),
    queryFn: async () => (await getBackend()).listDatabases(id),
  }),
  schemas: (id: string, database: string) => ({
    queryKey: catalogKeys.schemas(id, database),
    queryFn: async () => (await getBackend()).listSchemas(id, database),
  }),
  tables: (id: string, database: string, schema: string | null) => ({
    queryKey: catalogKeys.tables(id, database, schema),
    queryFn: async () => (await getBackend()).listTables(id, database, schema),
  }),
  columns: (id: string, database: string, schema: string | null, table: string) => ({
    queryKey: catalogKeys.columns(id, database, schema, table),
    queryFn: async () => (await getBackend()).listColumns(id, { database, schema, table }),
  }),
};

export const connectionsQuery = {
  queryKey: catalogKeys.connections,
  queryFn: async () => (await getBackend()).listConnections(),
};

export function useConnections() {
  return useQuery(connectionsQuery);
}

/** 打开连接时写入缓存的服务器版本;没打开时为 undefined。 */
export function useServerInfo(id: string | null) {
  return useQuery<ServerInfo>({ queryKey: catalogKeys.server(id ?? ""), queryFn: skipToken });
}

export function useDatabases(id: string, enabled: boolean) {
  return useQuery({ ...catalogQueries.databases(id), enabled });
}

export function useSchemas(id: string, database: string, enabled: boolean) {
  return useQuery({ ...catalogQueries.schemas(id, database), enabled });
}

export function useTables(id: string, database: string, schema: string | null, enabled: boolean) {
  return useQuery({ ...catalogQueries.tables(id, database, schema), enabled });
}

/** 信息面板要的属性和建表语句;没选中表时不取。刷新连接时随 catalog 一起失效。 */
export function useTableDetails(id: string | null, table: TableRef | null) {
  return useQuery({
    queryKey: catalogKeys.details(id ?? "", table?.database ?? "", table?.schema ?? null, table?.table ?? ""),
    queryFn: id && table ? async () => (await getBackend()).tableDetails(id, table) : skipToken,
  });
}

/** SQL 助手要的表结构概要;没选库或连接没开时不取。刷新连接时随 catalog 一起失效。 */
export function useSchemaOutline(id: string, database: string | null, schema: string | null, enabled: boolean) {
  return useQuery({
    queryKey: catalogKeys.outline(id, database ?? "", schema),
    queryFn: enabled && database ? async () => (await getBackend()).schemaOutline(id, database, schema) : skipToken,
    staleTime: 60_000,
  });
}
