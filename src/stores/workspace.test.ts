// 记录标签页:改动后稍等一会儿才写、退出前马上写完、写不进去时只提醒一次、恢复完之前不写。
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { setBackendForTesting, type Backend } from "../lib/api";
import { createMockBackend } from "../lib/mockBackend";
import { resetStores } from "../test/renderApp";
import { queryTab, useTabs } from "./tabs";
import { useUi } from "./ui";
import { flushWorkspace, SAVE_DELAY_MS, scheduleSave, startRecording, useWorkspace } from "./workspace";

const draft = { ...queryTab("c1", { database: "shop", schema: null }, "查询 1"), id: "query:1" };

let backend: Backend;
let save: MockInstance<Backend["saveWorkspace"]>;
let stop: () => void;

beforeEach(() => {
  vi.useFakeTimers();
  resetStores();
  backend = createMockBackend();
  save = vi.spyOn(backend, "saveWorkspace");
  setBackendForTesting(backend);
});

afterEach(() => {
  stop?.();
  vi.useRealTimers();
});

const written = () => JSON.parse(save.mock.calls.at(-1)![0]) as { tabs: { id: string; sql?: string }[] };

describe("recording the open tabs", () => {
  it("does not write anything before the last record has been restored", async () => {
    useTabs.getState().open(draft);
    scheduleSave();
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(save).not.toHaveBeenCalled();
    expect(await flushWorkspace()).toBe(true);
    expect(save).not.toHaveBeenCalled();
  });

  it("waits until typing pauses, then writes once", async () => {
    stop = startRecording();
    useTabs.getState().open(draft);
    for (const sql of ["S", "SE", "SELECT 1;"]) {
      useWorkspace.getState().setDraft(draft.id, { sql });
      await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS / 2);
    }
    expect(save).not.toHaveBeenCalled();
    expect(useWorkspace.getState().pending).toBe(true);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
    expect(save).toHaveBeenCalledOnce();
    expect(written().tabs[0]).toMatchObject({ id: draft.id, sql: "SELECT 1;" });
    await vi.waitFor(() => expect(useWorkspace.getState().pending).toBe(false));
  });

  it("ignores changes that do not affect the record", async () => {
    stop = startRecording();
    useTabs.getState().setUnsaved(draft.id, true);
    useWorkspace.setState({ pending: false });
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(save).not.toHaveBeenCalled();
  });

  it("writes right away when asked to before quitting", async () => {
    stop = startRecording();
    useTabs.getState().open(draft);
    expect(await flushWorkspace()).toBe(true);
    expect(save).toHaveBeenCalledOnce();
    expect(useWorkspace.getState().pending).toBe(false);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(save).toHaveBeenCalledOnce();
  });

  it("says once when the record cannot be written, and reports failure on quit", async () => {
    save.mockRejectedValue(Object.assign(new Error("磁盘满了"), { code: "storage", fields: [] }));
    stop = startRecording();
    useTabs.getState().open(draft);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
    await vi.waitFor(() => expect(useUi.getState().notice?.message).toBe("没能记下打开的标签页：磁盘满了"));
    const first = useUi.getState().notice;
    useWorkspace.getState().setDraft(draft.id, { sql: "x" });
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
    expect(save).toHaveBeenCalledTimes(2);
    expect(useUi.getState().notice).toBe(first);
    expect(useWorkspace.getState().pending).toBe(true);
    expect(await flushWorkspace()).toBe(false);
  });

  it("stops recording once stopped", async () => {
    stop = startRecording();
    stop();
    useTabs.getState().open(draft);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
    expect(save).not.toHaveBeenCalled();
  });
});
