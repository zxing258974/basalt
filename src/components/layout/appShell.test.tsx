// 应用外壳:欢迎页、全局快捷键、工具栏、状态栏、标签栏和侧栏宽度。
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import { objectsTab, useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { renderApp, resetStores } from "../../test/renderApp";
import { connectionKey } from "../sidebar/treeKeys";
import { setPlatformForTesting } from "../../lib/platform";
import { NoticeArea } from "./NoticeArea";

const toolbar = () => within(screen.getByRole("banner"));
const statusBar = () => within(screen.getByRole("contentinfo"));
const selectConnection = (id: string) => act(() => useUi.getState().select(connectionKey(id)));

describe("welcome page", () => {
  it("leaves the connection list to the sidebar instead of repeating it", async () => {
    renderApp();
    expect(await screen.findByRole("treeitem", { name: "笔记" })).toBeInTheDocument();
    const main = within(screen.getByRole("main"));
    expect(main.queryByRole("region", { name: "连接" })).not.toBeInTheDocument();
    expect(main.queryByText("笔记")).not.toBeInTheDocument();
    expect(main.queryByText(/还没有连接/)).not.toBeInTheDocument();
  });

  it("invites creating the first connection when there is none", async () => {
    const { user } = renderApp(createMockBackend([]));
    expect(await screen.findByText(/还没有连接/)).toBeInTheDocument();
    await user.click(within(screen.getByRole("main")).getByRole("button", { name: "新建连接" }));
    expect(screen.getByRole("dialog", { name: "新建连接" })).toBeInTheDocument();
  });
});

describe("shortcuts", () => {
  it("opens the new-connection dialog with ⌘N", async () => {
    const { user } = renderApp();
    await user.keyboard("{Meta>}{Shift>}n{/Shift}{/Meta}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.keyboard("{Meta>}n{/Meta}");
    expect(screen.getByRole("dialog", { name: "新建连接" })).toBeInTheDocument();
  });

  it("closes the active tab with ⌘W", async () => {
    const { user } = renderApp();
    act(() => {
      useTabs.getState().open(objectsTab("mock-mysql", "shop", null));
      useTabs.getState().open(objectsTab("mock-mysql", "mysql", null));
    });
    await user.keyboard("{Meta>}w{/Meta}");
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["shop"]);
    await user.keyboard("{Meta>}w{/Meta}");
    await user.keyboard("{Meta>}w{/Meta}");
    expect(useTabs.getState().tabs).toEqual([]);
  });
});

describe("toolbar", () => {
  it("only enables the connection buttons once a connection is selected", async () => {
    const { user } = renderApp();
    await screen.findByRole("treeitem", { name: "笔记" });
    expect(toolbar().getByRole("button", { name: "打开连接" })).toBeDisabled();
    expect(toolbar().getByRole("button", { name: "编辑" })).toBeDisabled();

    selectConnection("mock-sqlite");
    await user.click(toolbar().getByRole("button", { name: "打开连接" }));
    await user.click(await toolbar().findByRole("button", { name: "关闭连接" }));
    expect(await toolbar().findByRole("button", { name: "打开连接" })).toBeEnabled();

    await user.click(toolbar().getByRole("button", { name: "编辑" }));
    expect(screen.getByRole("dialog", { name: "编辑连接" })).toBeInTheDocument();
  });

  it("refreshes the connection list; a failure shows up in the tree", async () => {
    const backend = createMockBackend();
    const { user } = renderApp(backend);
    await screen.findByRole("treeitem", { name: "笔记" });
    vi.spyOn(backend, "listConnections").mockRejectedValue({ code: "io", message: "读不到配置", fields: [] });

    await user.click(toolbar().getByRole("button", { name: "刷新" }));
    expect(await within(screen.getByRole("tree")).findByText("读不到配置")).toBeInTheDocument();
  });

  it("reports a failed close in the status bar until dismissed", async () => {
    const backend = createMockBackend();
    await backend.openConnection("mock-sqlite");
    vi.spyOn(backend, "closeConnection").mockRejectedValue({ code: "io", message: "会话已失效", fields: [] });
    const { user } = renderApp(backend);
    selectConnection("mock-sqlite");

    await user.click(await toolbar().findByRole("button", { name: "关闭连接" }));
    expect(await statusBar().findByRole("alert")).toHaveTextContent("关闭连接失败：会话已失效");
    await user.click(statusBar().getByRole("button", { name: "关闭提示" }));
    expect(statusBar().queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("status bar", () => {
  it("describes the selected connection", async () => {
    renderApp();
    expect(await statusBar().findByText("3 个连接 · 0 个已打开")).toBeInTheDocument();
    expect(statusBar().getByText("未选择连接")).toBeInTheDocument();
    selectConnection("mock-pg");
    expect(statusBar().getByText("FOLIO 测试库")).toBeInTheDocument();
    expect(statusBar().getByText("folio@10.0.8.21:5432")).toBeInTheDocument();
    expect(statusBar().getByText("未连接")).toBeInTheDocument();
  });

  describe("notice", () => {
    beforeEach(() => {
      resetStores();
      vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    it("disappears on its own after a few seconds", () => {
      render(<NoticeArea />);
      act(() => useUi.getState().notify("复制连接失败：找不到连接"));
      expect(screen.getByRole("alert")).toBeInTheDocument();
      act(() => {
        vi.advanceTimersByTime(6000);
      });
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });
});

describe("tab bar", () => {
  it("activates tabs by click or keyboard and closes them", async () => {
    const { user } = renderApp();
    act(() => {
      useTabs.getState().open(objectsTab("mock-mysql", "shop", null));
      useTabs.getState().open(objectsTab("mock-mysql", "mysql", null));
    });
    const tabs = within(screen.getByRole("tablist", { name: "打开的标签页" }));
    await waitFor(() => expect(tabs.getByRole("tab", { name: /shop/ })).toHaveAttribute("title", "本地 MySQL / shop"));

    await user.click(tabs.getByRole("tab", { name: /shop/ }));
    expect(tabs.getByRole("tab", { name: /shop/ })).toHaveAttribute("aria-selected", "true");
    tabs.getByRole("tab", { name: /mysql/ }).focus();
    await user.keyboard("{Enter}");
    expect(useTabs.getState().activeId).toBe(objectsTab("mock-mysql", "mysql", null).id);

    fireEvent(tabs.getByRole("tab", { name: /shop/ }), new MouseEvent("auxclick", { bubbles: true, button: 1 }));
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["mysql"]);
    await user.click(tabs.getByRole("button", { name: "关闭 mysql" }));
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });
});

describe("sidebar width", () => {
  beforeEach(() => {
    localStorage.clear();
    HTMLElement.prototype.setPointerCapture = vi.fn();
  });

  const resizer = () => screen.getByRole("separator", { name: "调整侧栏宽度" });

  it.each([
    [null, 260],
    ["320", 320],
    ["9999", 480],
    ["abc", 260],
  ])("starts from the saved width %s", (saved, width) => {
    if (saved !== null) localStorage.setItem("basalt.sidebarWidth", saved);
    renderApp();
    expect(resizer()).toHaveAttribute("aria-valuenow", String(width));
  });

  it("follows a drag, remembers it, and resets on double click", async () => {
    const { user } = renderApp();
    fireEvent.pointerMove(resizer(), { clientX: 400 });
    expect(resizer()).toHaveAttribute("aria-valuenow", "260");

    fireEvent.pointerDown(resizer(), { clientX: 100, pointerId: 1 });
    fireEvent.pointerMove(resizer(), { clientX: 160 });
    fireEvent.pointerUp(resizer());
    expect(resizer()).toHaveAttribute("aria-valuenow", "320");
    expect(localStorage.getItem("basalt.sidebarWidth")).toBe("320");

    fireEvent.pointerDown(resizer(), { clientX: 100, pointerId: 1 });
    fireEvent.pointerMove(resizer(), { clientX: -500 });
    fireEvent.pointerUp(resizer());
    expect(resizer()).toHaveAttribute("aria-valuenow", "200");

    await user.dblClick(resizer());
    await waitFor(() => expect(resizer()).toHaveAttribute("aria-valuenow", "260"));
    expect(localStorage.getItem("basalt.sidebarWidth")).toBe("260");
  });
});

describe("on Windows", () => {
  afterEach(() => setPlatformForTesting("mac"));

  it("shows Ctrl shortcuts and answers to Ctrl instead of ⌘", async () => {
    setPlatformForTesting("windows");
    const { user } = renderApp();
    expect(await screen.findByText("Ctrl+N")).toBeInTheDocument();
    expect(screen.getByText("Ctrl+Enter")).toBeInTheDocument();
    await user.keyboard("{Meta>}n{/Meta}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.keyboard("{Control>}n{/Control}");
    const dialog = within(screen.getByRole("dialog", { name: "新建连接" }));
    await user.click(dialog.getByText("MySQL", { exact: true }));
    expect(dialog.getByLabelText("把密码保存在本机")).toBeInTheDocument();
  });
});
