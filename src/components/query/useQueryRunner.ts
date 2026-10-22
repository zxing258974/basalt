// 一条一条地执行语句:每条执行完就显示结果,出错或取消就停,后面的不再执行。
// 结果完整地存在后端;不再显示的(重新执行、换掉了)让后端放掉。
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import type { QueryScope } from "../../lib/queryTypes";
import { errorMessage } from "../../lib/types";
import { useTabs, type QueryTab } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { IDLE, type RunEntry, type RunState } from "./runModel";
import type { SqlStatement } from "./splitStatements";

/** 改了表结构的语句:执行完让对象树重新读这个连接的元数据。 */
const DDL = /^\s*(create|drop|alter|rename|truncate)\b/i;

const sameScope = (a: QueryScope, b: QueryScope) => a.database === b.database && a.schema === b.schema;

function replaceLast(state: RunState, entry: RunEntry): RunState {
  return { ...state, entries: [...state.entries.slice(0, -1), entry] };
}

const resultIds = (entries: readonly RunEntry[]) =>
  entries.flatMap((e) => e.outcome?.resultSets.map((s) => s.id) ?? []);

function release(editorId: string, ids: number[]) {
  if (ids.length === 0) return;
  void getBackend()
    .then((backend) => backend.releaseResults(editorId, ids))
    .catch((error: unknown) => console.warn("释放查询结果失败", error));
}

export function useQueryRunner(tab: QueryTab) {
  const queryClient = useQueryClient();
  const updateQuery = useTabs((s) => s.updateQuery);
  const [run, setRun] = useState<RunState>(IDLE);
  const shown = useRef<RunState>(IDLE);
  const running = useRef(false);
  const cancelled = useRef(false);
  // 执行中途标签页的库可能变(USE 之后),下一条要带上新的。
  const scope = useRef(tab.scope);
  useEffect(() => {
    scope.current = tab.scope;
  }, [tab.scope]);
  // 标签页关了:后面的语句一条也不再发(否则后端会为这个已关闭的标签页再开一条连接)。
  useEffect(
    () => () => {
      cancelled.current = true;
    },
    [],
  );

  const execute = useCallback(
    /** 已经在执行(或没有语句)时什么都不做,返回 null。 */
    async (statements: SqlStatement[]): Promise<RunState | null> => {
      if (running.current || statements.length === 0) return null;
      running.current = true;
      cancelled.current = false;
      let state: RunState = { running: true, total: statements.length, entries: [], cancelRequested: false };
      const commit = (next: RunState) => {
        state = next;
        shown.current = next;
        setRun(next);
      };
      release(tab.id, resultIds(shown.current.entries));
      commit(state);
      const backend = await getBackend();
      for (const statement of statements) {
        if (cancelled.current) break;
        commit({ ...state, entries: [...state.entries, { sql: statement.text, outcome: null, failure: null }] });
        try {
          const outcome = await backend.runStatement(tab.id, tab.connectionId, scope.current, statement.text);
          commit(replaceLast(state, { sql: statement.text, outcome, failure: null }));
          if (!sameScope(outcome.scope, scope.current)) {
            scope.current = outcome.scope;
            updateQuery(tab.id, { scope: outcome.scope });
          }
          if (outcome.status !== "ok") break;
        } catch (error) {
          commit(replaceLast(state, { sql: statement.text, outcome: null, failure: errorMessage(error) }));
          break;
        }
      }
      running.current = false;
      commit({ ...state, running: false, cancelRequested: cancelled.current });
      if (statements.some((s) => DDL.test(s.text))) {
        void queryClient.invalidateQueries({ queryKey: catalogKeys.connection(tab.connectionId) });
      }
      void queryClient.invalidateQueries({ queryKey: ["history"] });
      return state;
    },
    [queryClient, tab.connectionId, tab.id, updateQuery],
  );

  /**
   * 重新执行产生第 `index` 条结果的语句,换上新结果(改过结果、提交以后)。
   * 回到那条语句当时所在的库执行;标签页的库不跟着变,下一次执行时连接自己会切回来。
   */
  const rerun = useCallback(
    async (index: number) => {
      const entry = shown.current.entries[index];
      if (running.current || !entry?.outcome) return;
      running.current = true;
      const update = (next: RunState) => {
        shown.current = next;
        setRun(next);
      };
      update({ ...shown.current, running: true, cancelRequested: false });
      let replaced: RunEntry;
      try {
        const outcome = await (
          await getBackend()
        ).runStatement(tab.id, tab.connectionId, entry.outcome.scope, entry.sql);
        replaced = { sql: entry.sql, outcome, failure: null };
      } catch (error) {
        replaced = { sql: entry.sql, outcome: null, failure: errorMessage(error) };
      }
      running.current = false;
      const entries = shown.current.entries.map((e, i) => (i === index ? replaced : e));
      update({ ...shown.current, entries, running: false });
      release(tab.id, resultIds([entry]));
      void queryClient.invalidateQueries({ queryKey: ["history"] });
    },
    [queryClient, tab.connectionId, tab.id],
  );

  /** 停在当前这条:让数据库中止它,后面的不再执行。 */
  const cancel = useCallback(async () => {
    if (!running.current || cancelled.current) return;
    cancelled.current = true;
    setRun((current) => ({ ...current, cancelRequested: true }));
    try {
      await (await getBackend()).cancelStatement(tab.id);
    } catch (error) {
      // 取消失败没有专属位置显示,交给状态栏提示;语句照常跑完。
      useUi.getState().notify(`取消失败：${errorMessage(error)}`);
    }
  }, [tab.id]);

  return { run, execute, rerun, cancel };
}
