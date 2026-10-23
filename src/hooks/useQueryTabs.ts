// 打开查询标签页:新建的,或者对象树里保存的查询;Redis 连接开的是命令行。连接没打开时先打开。
import { useCallback } from "react";
import { getBackend } from "../lib/api";
import type { QueryScope, SavedQueryRef } from "../lib/queryTypes";
import { errorMessage, type ConnectionConfig, type ConnectionView } from "../lib/types";
import { nextQueryTitle, queryTab, redisConsoleTab, sameSavedQuery, useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";
import { useConnectionActions } from "./useConnectionActions";

/** 没指定库时用哪个:连接配置里的默认库;PostgreSQL 没写就是 postgres,SQLite 是 main。 */
export function defaultScope(connection: ConnectionConfig): QueryScope {
  if (connection.driver === "sqlite") return { database: "main", schema: null };
  const database = connection.endpoint.kind === "network" ? (connection.endpoint.database ?? null) : null;
  if (connection.driver === "postgres") return { database: database ?? "postgres", schema: null };
  return { database, schema: null };
}

/** Redis 连接配置里的默认库号,没写或写错是 0。 */
export function defaultRedisDb(connection: ConnectionConfig): number {
  const db = connection.endpoint.kind === "network" ? Number(connection.endpoint.database ?? 0) : 0;
  return Number.isInteger(db) && db >= 0 ? db : 0;
}

/** 正在读出内容的保存的查询:快速双击两次时不会开出两个标签页。 */
const opening = new Set<string>();
const refKey = (ref: SavedQueryRef) => JSON.stringify([ref.connectionId, ref.database, ref.schema, ref.name]);

export function useQueryTabs() {
  const { open } = useConnectionActions();

  /** 连接已打开或刚打开成功返回 true;要先输密码(密码框接手)或打开失败返回 false。 */
  const ensureOpen = useCallback(
    async (connection: ConnectionView) => {
      try {
        return await open(connection);
      } catch (error) {
        useUi.getState().notify(`打开连接失败：${errorMessage(error)}`);
        return false;
      }
    },
    [open],
  );

  const openNewQuery = useCallback(
    async (connection: ConnectionView, location?: Partial<QueryScope>) => {
      if (!(await ensureOpen(connection))) return;
      const fallback = defaultScope(connection);
      const scope: QueryScope = {
        database: location?.database ?? fallback.database,
        schema: location?.schema ?? null,
      };
      const { tabs, open: openTab } = useTabs.getState();
      openTab(queryTab(connection.id, scope, nextQueryTitle(tabs)));
    },
    [ensureOpen],
  );

  /** 已经开着就切过去,否则读出内容新开一个标签页。 */
  const openSavedQuery = useCallback(
    async (connection: ConnectionView, ref: SavedQueryRef) => {
      const { tabs, activate, open: openTab } = useTabs.getState();
      const existing = tabs.find((t) => t.kind === "query" && sameSavedQuery(t.saved, ref));
      if (existing) {
        activate(existing.id);
        return;
      }
      const key = refKey(ref);
      if (opening.has(key)) return;
      opening.add(key);
      try {
        if (!(await ensureOpen(connection))) return;
        const sql = await (await getBackend()).readSavedQuery(ref);
        openTab(queryTab(connection.id, { database: ref.database, schema: ref.schema }, ref.name, ref, sql));
      } catch (error) {
        useUi.getState().notify(`打开查询「${ref.name}」失败：${errorMessage(error)}`);
      } finally {
        opening.delete(key);
      }
    },
    [ensureOpen],
  );

  /** 新开一个 Redis 命令行,在库 `db`(没给就是连接的默认库)。 */
  const openRedisConsole = useCallback(
    async (connection: ConnectionView, db?: number) => {
      if (!(await ensureOpen(connection))) return;
      const { tabs, open: openTab } = useTabs.getState();
      openTab(redisConsoleTab(connection.id, db ?? defaultRedisDb(connection), tabs));
    },
    [ensureOpen],
  );

  return { openNewQuery, openSavedQuery, openRedisConsole };
}
