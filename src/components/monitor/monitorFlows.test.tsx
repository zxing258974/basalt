// 服务器监控页:从连接的右键菜单打开,指标和会话列表,筛选,终止会话,读取失败,停止轮询。
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { setBackendForTesting, type Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { CommandError } from "../../lib/types";
import { monitorTab, useTabs } from "../../stores/tabs";
import { renderApp } from "../../test/renderApp";
import { useServerMonitor } from "./useServerMonitor";

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const rowOf = async (name: string) => (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;

async function showMonitor(connectionId = "mock-mysql", backend: Backend = createMockBackend()) {
  await backend.openConnection(connectionId, "pw");
  const context = renderApp(backend);
  act(() => useTabs.getState().open(monitorTab(connectionId)));
  await screen.findByRole("table");
  return context;
}

const sessionIds = () =>
  within(screen.getByRole("table"))
    .getAllByRole("row")
    .slice(1)
    .map((row) => within(row).getAllByRole("cell")[0]?.textContent?.replace("Basalt", ""));

describe("opening", () => {
  it("opens from the connection menu once the connection is open", async () => {
    const { user } = renderApp();
    await user.dblClick(await rowOf("开发"));
    await user.pointer({ keys: "[MouseRight]", target: await rowOf("本地 MySQL") });
    expect(screen.getByRole("menuitem", { name: "服务器监控" })).toBeDisabled();
    await user.keyboard("{Escape}");

    await user.dblClick(await rowOf("本地 MySQL"));
    await tree().findByRole("treeitem", { name: "shop" });
    await user.pointer({ keys: "[MouseRight]", target: await rowOf("本地 MySQL") });
    await user.click(screen.getByRole("menuitem", { name: "服务器监控" }));
    expect(useTabs.getState().tabs.map((t) => t.id)).toEqual(["monitor:mock-mysql"]);
    expect(await screen.findByRole("heading", { name: "服务器监控" })).toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "查询 / 秒" })).toBeInTheDocument();

    // 再点一次还是同一个标签页。
    await user.pointer({ keys: "[MouseRight]", target: await rowOf("本地 MySQL") });
    await user.click(screen.getByRole("menuitem", { name: "服务器监控" }));
    expect(useTabs.getState().tabs).toHaveLength(1);
  });

  it("is not offered for SQLite", async () => {
    const { user } = renderApp();
    await user.pointer({ keys: "[MouseRight]", target: await rowOf("笔记") });
    expect(screen.getByRole("menuitem", { name: "刷新" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "服务器监控" })).not.toBeInTheDocument();
  });

  it("asks to reopen a closed connection", async () => {
    renderApp();
    act(() => useTabs.getState().open(monitorTab("mock-mysql")));
    expect(await screen.findByText("连接已关闭")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "立即刷新" })).toBeDisabled();
  });
});

describe("metrics", () => {
  it("shows MySQL's throughput, connections, cache, network and uptime", async () => {
    const { user } = await showMonitor();
    const tile = (name: string) => screen.getByRole("region", { name });
    expect(tile("连接")).toHaveTextContent("13 / 151");
    expect(tile("连接")).toHaveTextContent("在执行 3");
    expect(within(tile("连接")).getByRole("meter")).toHaveAttribute("aria-valuenow", "13");
    expect(within(tile("连接")).getByRole("meter")).toHaveAttribute("aria-valuetext", "13 / 151");
    expect(tile("缓存命中率")).toHaveTextContent("InnoDB 缓冲池");
    expect(tile("缓存命中率")).toHaveTextContent(/9\d\.\d\d%/);
    expect(tile("慢查询")).toHaveTextContent("37");
    expect(screen.getByText(/^已运行 3 天 4 小时 · 更新于 \d\d:\d\d:\d\d$/)).toBeInTheDocument();
    // 一份快照还算不出速率。
    expect(tile("查询 / 秒")).toHaveTextContent("—");
    expect(screen.getByRole("region", { name: "接收" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "立即刷新" }));
    await waitFor(() => expect(tile("查询 / 秒")).not.toHaveTextContent("—"));
    expect(within(tile("查询 / 秒")).getByRole("img", { name: "查询 / 秒最近 1 次的走势" })).toBeInTheDocument();
  });

  it("shows transactions instead of queries for PostgreSQL", async () => {
    await showMonitor("mock-pg");
    expect(screen.getByRole("region", { name: "事务提交 / 秒" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "缓存命中率" })).toHaveTextContent("共享缓冲区");
    expect(screen.queryByRole("region", { name: "查询 / 秒" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "接收" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "慢查询" })).not.toBeInTheDocument();
  });

  it("explains when the account can only see part of the server", async () => {
    const backend = createMockBackend();
    const snapshot = backend.serverSnapshot.bind(backend);
    backend.serverSnapshot = async (id) => {
      const real = await snapshot(id);
      const hidden = {
        ...real.processes[0]!,
        id: 77,
        state: null,
        detail: null,
        seconds: null,
        query: null,
        activity: "unknown" as const,
      };
      return { ...real, processes: [...real.processes, hidden], limited: "看不到别人在做什么。" };
    };
    await showMonitor("mock-pg", backend);
    expect(screen.getByText("看不到别人在做什么。")).toBeInTheDocument();
    const row = screen.getByRole("button", { name: "断开会话 77" }).closest("tr")!;
    expect(row).toHaveTextContent("看不到");
    expect(within(row).getByRole("button", { name: "终止会话 77 正在执行的语句" })).toBeDisabled();
    expect(within(row).getByRole("button", { name: "断开会话 77" })).toBeEnabled();
  });
});

describe("sessions", () => {
  it("lists running sessions first and filters them", async () => {
    const { user } = await showMonitor();
    expect(screen.getByText("共 13 个 · 在执行 3")).toBeInTheDocument();
    expect(sessionIds().slice(0, 3)).toEqual(["1184", "1191", "1210"]);
    // 事件调度器是服务器自己的线程:排最后,不能终止。
    expect(sessionIds().at(-1)).toBe("5");
    expect(screen.getByRole("button", { name: "断开会话 5" })).toBeDisabled();
    expect(sessionIds()).toHaveLength(13);

    await user.click(screen.getByRole("checkbox", { name: "隐藏空闲" }));
    expect(sessionIds()).toEqual(["1184", "1191", "1210"]);

    await user.type(screen.getByRole("textbox", { name: "筛选会话" }), "metadata lock");
    expect(sessionIds()).toEqual(["1191"]);
    await user.clear(screen.getByRole("textbox", { name: "筛选会话" }));
    await user.type(screen.getByRole("textbox", { name: "筛选会话" }), "nobody");
    expect(screen.getByText("没有符合筛选条件的会话")).toBeInTheDocument();
  });

  it("marks Basalt's own connection and won't let it be killed", async () => {
    await showMonitor();
    const own = within(screen.getByRole("table")).getByText("Basalt").closest("tr")!;
    expect(own).toHaveTextContent("1210");
    expect(within(own).getByRole("button", { name: "断开会话 1210" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "终止会话 1203 正在执行的语句" })).toBeDisabled();
  });

  it("stops a statement after confirming", async () => {
    const { user } = await showMonitor();
    await user.click(screen.getByRole("button", { name: "终止会话 1184 正在执行的语句" }));
    const dialog = within(screen.getByRole("dialog", { name: "终止会话 1184 的语句？" }));
    expect(dialog.getByText("app @ 10.0.8.31:40112 · shop")).toBeInTheDocument();
    expect(dialog.getByText(/^SELECT o\.id, c\.name/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "终止语句" }));

    expect(await screen.findByText("已终止会话 1184 的语句。")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("共 13 个 · 在执行 2")).toBeInTheDocument());
  });

  it("drops a session, and shows the database's error when it fails", async () => {
    const backend = createMockBackend();
    const { user } = await showMonitor("mock-mysql", backend);
    await user.click(screen.getByRole("button", { name: "断开会话 1191" }));
    await user.click(screen.getByRole("button", { name: "断开连接" }));
    await waitFor(() => expect(sessionIds()).not.toContain("1191"));

    const failure: CommandError = { code: "database", message: "Unknown thread id: 1184", fields: [] };
    vi.spyOn(backend, "killProcess").mockRejectedValue(failure);
    await user.click(screen.getByRole("button", { name: "断开会话 1184" }));
    await user.click(screen.getByRole("button", { name: "断开连接" }));
    const dialog = within(screen.getByRole("dialog", { name: "断开会话 1184？" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("Unknown thread id: 1184");
  });
});

describe("failures", () => {
  it("shows the error and retries", async () => {
    const backend = createMockBackend();
    await backend.openConnection("mock-mysql", "pw");
    const snapshot = backend.serverSnapshot.bind(backend);
    const denied: CommandError = {
      code: "database",
      message: "Access denied; you need the PROCESS privilege",
      fields: [],
    };
    backend.serverSnapshot = vi.fn().mockRejectedValueOnce(denied).mockImplementation(snapshot);
    const { user } = renderApp(backend);
    act(() => useTabs.getState().open(monitorTab("mock-mysql")));
    expect(await screen.findByText("读取服务器状态失败")).toBeInTheDocument();
    expect(screen.getByText(/PROCESS privilege/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("table")).toBeInTheDocument();
  });

  it("keeps the last numbers when a refresh fails", async () => {
    const backend = createMockBackend();
    const { user } = await showMonitor("mock-mysql", backend);
    const lost: CommandError = { code: "database", message: "Lost connection to MySQL server", fields: [] };
    vi.spyOn(backend, "serverSnapshot").mockRejectedValue(lost);
    await user.click(screen.getByRole("button", { name: "立即刷新" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("刷新失败：Lost connection to MySQL server");
    expect(screen.getByRole("table")).toBeInTheDocument();
  });
});

describe("polling", () => {
  it("says when it is paused and which interval is in use", async () => {
    const { user } = await showMonitor();
    expect(screen.getByRole("radio", { name: "2 秒" })).toBeChecked();
    await user.click(screen.getByRole("radio", { name: "10 秒" }));
    expect(screen.getByRole("radio", { name: "10 秒" })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "暂停" }));
    expect(screen.getByRole("button", { name: "继续" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText(/已暂停，停在 \d\d:\d\d:\d\d$/)).toBeInTheDocument();
  });

  it("asks again on every tick and stops when paused", async () => {
    vi.useFakeTimers();
    try {
      const backend = createMockBackend();
      await act(() => backend.openConnection("mock-mysql", "pw"));
      const spy = vi.spyOn(backend, "serverSnapshot");
      setBackendForTesting(backend);
      const client = new QueryClient();
      const wrapper = ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      );
      const initialProps: { intervalMs: number | null } = { intervalMs: 2000 };
      const { result, rerender } = renderHook(
        ({ intervalMs }: { intervalMs: number | null }) =>
          useServerMonitor({ connectionId: "mock-mysql", enabled: true, intervalMs }),
        { wrapper, initialProps },
      );
      await act(() => vi.advanceTimersByTimeAsync(500));
      expect(spy).toHaveBeenCalledTimes(1);
      await act(() => vi.advanceTimersByTimeAsync(2000));
      expect(spy).toHaveBeenCalledTimes(2);
      expect(result.current.history.points).toHaveLength(1);

      rerender({ intervalMs: null });
      await act(() => vi.advanceTimersByTimeAsync(10_000));
      expect(spy).toHaveBeenCalledTimes(2);
      client.clear();
    } finally {
      vi.useRealTimers();
    }
  });
});
