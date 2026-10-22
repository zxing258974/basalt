import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../App";
import { connectionKey, databaseKey, folderKey } from "../components/sidebar/treeKeys";
import { setBackendForTesting, type Backend } from "../lib/api";
import { createMockBackend } from "../lib/mockBackend";
import type { CommandError, ConnectionConfig, ConnectionView } from "../lib/types";
import { objectsTab, useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";
import { resetStores } from "../test/renderApp";
import { catalogKeys } from "./useCatalog";
import { useConnectionActions } from "./useConnectionActions";

const saved: ConnectionView = {
  id: "saved",
  name: "本地",
  group: "开发",
  driver: "mysql",
  endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root" },
  savePassword: true,
  isOpen: false,
};

const remote: ConnectionConfig = {
  id: "remote",
  name: "远程",
  driver: "postgres",
  endpoint: { kind: "network", host: "10.0.0.5", port: 5432, username: "app" },
  savePassword: false,
};

const failure = (code: string, message: string): CommandError => ({ code, message, fields: [] });

function setup(backend: Backend = createMockBackend([saved, remote])) {
  setBackendForTesting(backend);
  const queryClient = createQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  const { result } = renderHook(() => useConnectionActions(), { wrapper });
  return { actions: () => result.current, backend, queryClient };
}

beforeEach(resetStores);

describe("open", () => {
  it("does nothing for a connection that is already open", async () => {
    const { actions, backend } = setup();
    const openConnection = vi.spyOn(backend, "openConnection");
    expect(await actions().open({ ...saved, isOpen: true })).toBe(true);
    expect(openConnection).not.toHaveBeenCalled();
  });

  it("asks for the password first when none is saved", async () => {
    const { actions } = setup();
    expect(await actions().open(remote)).toBe(false);
    expect(useUi.getState().passwordPrompt).toEqual({ connection: remote });
  });

  it("caches the server version and reveals the connection with its folder", async () => {
    const { actions, queryClient } = setup();
    expect(await act(() => actions().open(saved))).toBe(true);
    expect(queryClient.getQueryData(catalogKeys.server("saved"))).toEqual({ product: "MySQL", version: "8.4.3" });
    expect(useUi.getState().expanded).toMatchObject({ [connectionKey("saved")]: true, [folderKey("开发")]: true });
  });

  it("brings the password prompt back with the reason when the password is wrong", async () => {
    const backend = createMockBackend([remote]);
    vi.spyOn(backend, "openConnection").mockRejectedValue(failure("authentication", "密码错误"));
    const { actions } = setup(backend);
    expect(await actions().open(remote, { password: "wrong" })).toBe(false);
    expect(useUi.getState().passwordPrompt).toEqual({ connection: remote, error: "密码错误" });
  });

  it("never asks anything when reconnecting at startup, and says why instead", async () => {
    for (const code of ["authentication", "ssh_secret_required", "host_key_unknown"]) {
      resetStores();
      const backend = createMockBackend([saved]);
      vi.spyOn(backend, "openConnection").mockRejectedValue(failure(code, "要用户来"));
      const confirm = vi.spyOn(useUi.getState(), "confirmHostKey");
      const { actions } = setup(backend);
      await expect(actions().open(saved, undefined, { prompt: false })).rejects.toMatchObject({ code });
      expect(useUi.getState().passwordPrompt).toBeNull();
      expect(confirm).not.toHaveBeenCalled();
    }
    resetStores();
    const { actions } = setup();
    expect(await actions().open(remote, undefined, { prompt: false })).toBe(false);
    expect(useUi.getState().passwordPrompt).toBeNull();
  });

  it("lets other failures reach the caller", async () => {
    const backend = createMockBackend([saved]);
    vi.spyOn(backend, "openConnection").mockRejectedValue(failure("timeout", "连接超时"));
    const { actions } = setup(backend);
    await expect(actions().open(saved)).rejects.toMatchObject({ code: "timeout" });
  });
});

describe("openWithFeedback", () => {
  it("keeps the node expanded and shows why it failed", async () => {
    const backend = createMockBackend([saved]);
    vi.spyOn(backend, "openConnection").mockRejectedValue(failure("timeout", "连接超时"));
    const { actions } = setup(backend);
    expect(await act(() => actions().openWithFeedback(saved))).toBe(true);
    expect(useUi.getState().connectionErrors).toEqual({ saved: "连接超时" });
    expect(useUi.getState().expanded[connectionKey("saved")]).toBe(true);
  });

  it("clears the previous failure on the next attempt", async () => {
    const { actions } = setup();
    useUi.getState().setConnectionError("saved", "连接超时");
    await act(() => actions().openWithFeedback(saved));
    expect(useUi.getState().connectionErrors).toEqual({});
  });
});

describe("closing and removing", () => {
  async function openWithUi() {
    const context = setup();
    await act(() => context.actions().open(saved));
    useTabs.getState().open(objectsTab("saved", "shop", null));
    useUi.getState().setExpanded(databaseKey("saved", "shop"), true);
    useUi.getState().setConnectionError("saved", "旧错误");
    return context;
  }

  function expectForgotten() {
    expect(useTabs.getState().tabs).toEqual([]);
    expect(useUi.getState().expanded[databaseKey("saved", "shop")]).toBeUndefined();
    expect(useUi.getState().connectionErrors).toEqual({});
  }

  it("closes the session and cleans up its tabs and subtree", async () => {
    const { actions, backend } = await openWithUi();
    await act(() => actions().close("saved"));
    expect((await backend.listConnections())[0]?.isOpen).toBe(false);
    expectForgotten();
  });

  it("deletes the connection and cleans up the same way", async () => {
    const { actions, backend } = await openWithUi();
    await act(() => actions().remove("saved"));
    expect((await backend.listConnections()).map((c) => c.id)).toEqual(["remote"]);
    expectForgotten();
  });

  it("cleans up after a save that dropped the session", async () => {
    const { actions, backend } = await openWithUi();
    await backend.closeConnection("saved");
    await act(() => actions().afterSave("saved"));
    expectForgotten();
  });
});

describe("other actions", () => {
  it("duplicates and refreshes the connection list", async () => {
    const { actions, queryClient } = setup();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const copy = await act(() => actions().duplicate("saved"));
    expect(copy.name).toBe("本地 副本");
    expect(invalidate).toHaveBeenCalledWith({ queryKey: catalogKeys.connections });
  });

  it("remembers a password through the backend", async () => {
    const { actions, backend } = setup();
    const rememberPassword = vi.spyOn(backend, "rememberPassword");
    await act(() => actions().rememberPassword("remote", "pw"));
    expect(rememberPassword).toHaveBeenCalledWith("remote", "pw");
  });

  it("refreshes one connection's metadata or just the list", async () => {
    const { actions, queryClient } = setup();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    await act(() => actions().refresh("saved"));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: catalogKeys.connection("saved") });

    invalidate.mockClear();
    await act(() => actions().refresh());
    expect(invalidate).toHaveBeenCalledTimes(1);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: catalogKeys.connections });
  });

  it("reports a failed action in the status bar", async () => {
    const { actions } = setup();
    actions().withNotice("复制连接", () => Promise.reject(new Error("找不到连接")));
    await vi.waitFor(() => expect(useUi.getState().notice?.message).toBe("复制连接失败：找不到连接"));
  });
});
