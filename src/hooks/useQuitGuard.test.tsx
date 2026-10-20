import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setBackendForTesting, type Backend } from "../lib/api";
import { createMockBackend } from "../lib/mockBackend";
import { queryTab, tableDataTab, useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";
import { resetStores } from "../test/renderApp";
import { useQuitGuard } from "./useQuitGuard";

const orders = tableDataTab("c1", "shop", null, "orders");
const report = queryTab("c1", { database: "shop", schema: null }, "查询 1");

/** 记下后端收到的调用;`requestQuit` 模拟用户按 ⌘Q 或点关闭按钮。 */
function fakeBackend() {
  let onQuit: (() => void) | null = null;
  const unlisten = vi.fn();
  const backend: Backend = {
    ...createMockBackend(),
    setUnsavedWork: vi.fn(() => Promise.resolve()),
    quitApp: vi.fn(() => Promise.resolve()),
    onQuitRequested: vi.fn((handler: () => void) => {
      onQuit = handler;
      return Promise.resolve(unlisten);
    }),
  };
  const requestQuit = () => act(() => onQuit?.());
  return { backend, unlisten, requestQuit };
}

async function setup() {
  const fake = fakeBackend();
  setBackendForTesting(fake.backend);
  const view = renderHook(() => useQuitGuard());
  await waitFor(() => expect(fake.backend.onQuitRequested).toHaveBeenCalled());
  return { ...fake, ...view };
}

beforeEach(() => {
  resetStores();
  useTabs.setState({ tabs: [orders, report], activeId: report.id });
});

describe("useQuitGuard", () => {
  it("tells the backend whether anything is unsaved, only when that changes", async () => {
    const { backend } = await setup();
    await waitFor(() => expect(backend.setUnsavedWork).toHaveBeenLastCalledWith(false));

    act(() => useTabs.getState().setUnsaved(report.id, true));
    await waitFor(() => expect(backend.setUnsavedWork).toHaveBeenLastCalledWith(true));
    act(() => useTabs.getState().setPending(orders.id, 2));
    act(() => useTabs.getState().setUnsaved(report.id, false));
    act(() => useTabs.getState().setPending(orders.id, 0));
    await waitFor(() => expect(backend.setUnsavedWork).toHaveBeenLastCalledWith(false));
    expect(vi.mocked(backend.setUnsavedWork).mock.calls).toEqual([[false], [true], [false]]);
  });

  it("quits right away when nothing is unsaved", async () => {
    const { backend, requestQuit } = await setup();
    requestQuit();
    await waitFor(() => expect(backend.quitApp).toHaveBeenCalledOnce());
    expect(useUi.getState().discardPrompt).toBeNull();
  });

  it("asks first and quits only after the user gives up the changes", async () => {
    const { backend, requestQuit } = await setup();
    act(() => useTabs.getState().setPending(orders.id, 2));
    act(() => useTabs.getState().setUnsaved(report.id, true));
    requestQuit();

    const prompt = useUi.getState().discardPrompt;
    expect(prompt).toMatchObject({ count: 2, queries: 1, quit: true });
    expect(backend.quitApp).not.toHaveBeenCalled();
    act(() => prompt!.onConfirm());
    await waitFor(() => expect(backend.quitApp).toHaveBeenCalledOnce());
  });

  it("tells the user when quitting fails", async () => {
    const { backend, requestQuit } = await setup();
    vi.mocked(backend.quitApp).mockRejectedValueOnce(new Error("busy"));
    requestQuit();
    await waitFor(() => expect(useUi.getState().notice?.message).toBe("退出失败：busy"));
  });

  it("reports again on the next change after a failed report", async () => {
    const { backend } = await setup();
    await waitFor(() => expect(backend.setUnsavedWork).toHaveBeenCalledWith(false));
    vi.mocked(backend.setUnsavedWork).mockRejectedValueOnce(new Error("ipc down"));

    act(() => useTabs.getState().setUnsaved(report.id, true));
    await waitFor(() => expect(useUi.getState().notice?.message).toBe("退出时的未保存提醒可能不准：ipc down"));
    act(() => useTabs.getState().setPending(orders.id, 1));
    await waitFor(() => expect(vi.mocked(backend.setUnsavedWork).mock.calls).toEqual([[false], [true], [true]]));
  });

  it("stops listening and reporting once unmounted", async () => {
    const { backend, unlisten, unmount } = await setup();
    await waitFor(() => expect(backend.setUnsavedWork).toHaveBeenCalledOnce());
    unmount();
    await waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
    act(() => useTabs.getState().setUnsaved(report.id, true));
    await Promise.resolve();
    expect(backend.setUnsavedWork).toHaveBeenCalledOnce();
  });
});
