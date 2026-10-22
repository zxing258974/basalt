// 查询结果在前端只放看过的行:先显示随结果带回的第一批,滚到底再向后端要下一批;
// 点列头排序由后端排好整个结果,再从头取。
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { getBackend } from "../../lib/api";
import type { ResultSet } from "../../lib/queryTypes";
import { errorMessage, type CellValue, type SortSpec } from "../../lib/types";
import { batchSize, toResultSort } from "./resultModel";

interface Loaded {
  rows: CellValue[][];
  sort: SortSpec | null;
  loading: boolean;
  error: string | null;
  /** 换了排序、新顺序的行还没到:手上的行是旧顺序,不能再改(修改按行号记)。 */
  stale: boolean;
  /** 整批行被换掉(换了排序)一次加一:按行号记的修改要跟着作废。 */
  version: number;
}

export interface ResultRows extends Loaded {
  /** 后端还有没取过来的行。 */
  hasMore: boolean;
  /** 接着往下取一批;正在取、取完了、出过错或排序还没到时什么都不做。 */
  loadMore(): void;
  /** 出错后再试一次:换排序失败的从头取,往下取失败的接着取。 */
  retry(): void;
  changeSort(sort: SortSpec | null): void;
}

/** 结果集换了(重新执行)由调用方换 key 重新挂载,这里不跟。 */
export function useResultRows(editorId: string, set: ResultSet): ResultRows {
  const [state, setState] = useState<Loaded>({
    rows: set.rows,
    sort: null,
    loading: false,
    error: null,
    stale: false,
    version: 0,
  });
  const latest = useRef(state);
  useLayoutEffect(() => {
    latest.current = state;
  });
  /** 每换一次排序加一:换之前发出去的请求回来了也不再用。 */
  const generation = useRef(0);
  const busy = useRef(false);

  const fetchRows = useCallback(
    (offset: number, sort: SortSpec | null, apply: (rows: CellValue[][]) => (current: Loaded) => Loaded) => {
      const mine = generation.current;
      busy.current = true;
      setState((current) => ({ ...current, loading: true, error: null }));
      void getBackend()
        .then((backend) => backend.resultRows(editorId, set.id, offset, batchSize(offset), toResultSort(sort)))
        .then(
          (rows) => {
            if (mine !== generation.current) return;
            busy.current = false;
            setState((current) => ({ ...apply(rows)(current), loading: false }));
          },
          (error: unknown) => {
            if (mine !== generation.current) return;
            busy.current = false;
            setState((current) => ({ ...current, loading: false, error: errorMessage(error) }));
          },
        );
    },
    [editorId, set.id],
  );

  /** 按 `sort` 从头取,取到后换掉手上所有的行。 */
  const replace = useCallback(
    (sort: SortSpec) => {
      setState((current) => ({ ...current, sort, stale: true }));
      fetchRows(0, sort, (rows) => (current) => ({ ...current, rows, stale: false, version: current.version + 1 }));
    },
    [fetchRows],
  );

  const loadMore = useCallback(() => {
    const { rows, sort, error, stale } = latest.current;
    if (busy.current || error || stale || rows.length >= set.totalRows) return;
    fetchRows(rows.length, sort, (more) => (current) => ({ ...current, rows: [...current.rows, ...more] }));
  }, [fetchRows, set.totalRows]);

  const retry = useCallback(() => {
    const { sort, stale } = latest.current;
    if (busy.current) return;
    latest.current = { ...latest.current, error: null };
    if (stale && sort) replace(sort);
    else loadMore();
  }, [loadMore, replace]);

  const changeSort = useCallback(
    (sort: SortSpec | null) => {
      generation.current += 1;
      busy.current = false;
      // 回到不排序:第一批本来就在手上,不用再取。
      if (!sort) {
        setState((current) => ({
          rows: set.rows,
          sort: null,
          loading: false,
          error: null,
          stale: false,
          version: current.version + 1,
        }));
        return;
      }
      replace(sort);
    },
    [replace, set.rows],
  );

  return { ...state, hasMore: state.rows.length < set.totalRows, loadMore, retry, changeSort };
}
