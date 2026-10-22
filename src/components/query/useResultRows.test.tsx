// 结果的分批取行与后端排序:换排序时旧行作废、出错后重试从哪里接着取。
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setBackendForTesting, type Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ResultSet } from "../../lib/queryTypes";
import type { CellValue } from "../../lib/types";
import { useResultRows } from "./useResultRows";

afterEach(() => setBackendForTesting(null));

const int = (n: number): CellValue => ({ kind: "int", value: String(n) });

const set: ResultSet = {
  id: 9,
  columns: [{ name: "n", dataType: "int", source: null }],
  rows: [[int(1)], [int(2)]],
  totalRows: 4,
  truncated: false,
  readOnly: null,
};

/** 后端只换掉取行:每次调用由测试决定什么时候、以什么结果返回。 */
function backendWith(resultRows: Backend["resultRows"]) {
  const backend = createMockBackend();
  const spy = vi.spyOn(backend, "resultRows").mockImplementation(resultRows);
  setBackendForTesting(backend);
  return spy;
}

describe("useResultRows", () => {
  it("appends the next batch and stops at the end", async () => {
    const calls = backendWith((_editor, _id, offset) => Promise.resolve([[int(offset + 1)], [int(offset + 2)]]));
    const { result } = renderHook(() => useResultRows("tab", set));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.rows).toHaveLength(4));
    expect(calls).toHaveBeenCalledWith("tab", 9, 2, 1000, null);
    expect(result.current.hasMore).toBe(false);
    act(() => result.current.loadMore());
    expect(calls).toHaveBeenCalledTimes(1);
  });

  it("treats the shown rows as stale until the sorted ones arrive", async () => {
    let resolve: (rows: CellValue[][]) => void = () => {};
    const calls = backendWith(() => new Promise((done) => (resolve = done)));
    const { result } = renderHook(() => useResultRows("tab", set));
    act(() => result.current.changeSort({ column: "0:n", descending: true }));
    expect(result.current).toMatchObject({ stale: true, version: 0, rows: set.rows });
    await waitFor(() => expect(calls).toHaveBeenCalledTimes(1));
    act(() => result.current.loadMore());
    expect(calls).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolve([[int(4)], [int(3)]]);
      await Promise.resolve();
    });
    expect(result.current).toMatchObject({ stale: false, version: 1, rows: [[int(4)], [int(3)]] });
  });

  it("reloads from the top when a sort failed, instead of appending sorted rows to unsorted ones", async () => {
    const calls = backendWith(() => Promise.reject(new Error("排序失败")));
    const { result } = renderHook(() => useResultRows("tab", set));
    act(() => result.current.changeSort({ column: "0:n", descending: false }));
    await waitFor(() => expect(result.current.error).toBe("排序失败"));
    expect(result.current.stale).toBe(true);

    calls.mockImplementation(() => Promise.resolve([[int(1)], [int(2)]]));
    act(() => result.current.retry());
    await waitFor(() => expect(result.current.stale).toBe(false));
    expect(calls).toHaveBeenLastCalledWith("tab", 9, 0, 1000, { column: 0, descending: false });
    expect(result.current.error).toBeNull();
  });

  it("goes back to the first batch without asking when the sort is cleared", async () => {
    const calls = backendWith(() => Promise.resolve([[int(4)], [int(3)]]));
    const { result } = renderHook(() => useResultRows("tab", set));
    act(() => result.current.changeSort({ column: "0:n", descending: true }));
    await waitFor(() => expect(result.current.version).toBe(1));
    act(() => result.current.changeSort(null));
    expect(result.current).toMatchObject({ rows: set.rows, sort: null, version: 2, stale: false });
    expect(calls).toHaveBeenCalledTimes(1);
  });
});
