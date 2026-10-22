// 关掉 Basalt 再打开(mock 后端):上次的查询带着 SQL 马上回来,表数据页等连接自动连上再插回原位,
// 要输密码的连接不自动连、在状态栏说一声;之后打的字、开关的标签页稍等一会儿就记下来。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../lib/api";
import { createMockBackend } from "../lib/mockBackend";
import type { SavedTab } from "../lib/workspace";
import { objectsTab, tableDataTab, useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";
import { SAVE_DELAY_MS } from "../stores/workspace";
import { findEditor, installLayoutStubs, setSql } from "../test/editor";
import { renderApp } from "../test/renderApp";

installLayoutStubs();

const scope = { database: "shop", schema: null };
const scratch: SavedTab = {
  kind: "query",
  id: "query:scratch",
  title: "查询 3",
  connectionId: "mock-mysql",
  scope,
  saved: null,
  sql: "SELECT 42;",
  baselineSql: "",
};
const report: SavedTab = {
  kind: "query",
  id: "query:report",
  title: "日报",
  connectionId: "mock-mysql",
  scope,
  saved: { connectionId: "mock-mysql", database: "shop", schema: null, name: "日报" },
  sql: "SELECT 2;",
  baselineSql: "SELECT 1;",
};
const customers = tableDataTab("mock-mysql", "shop", null, "customers");
const folio = objectsTab("mock-pg", "folio", "public");

async function lastTime(tabs: SavedTab[], activeId: string | null, backend: Backend = createMockBackend()) {
  await backend.saveWorkspace(JSON.stringify({ version: 1, activeId, tabs }));
  return backend;
}

const ids = () => useTabs.getState().tabs.map((t) => t.id);
/** 最近一次写进记录的内容。 */
const recordedTabs = (save: { mock: { calls: [string][] } }) =>
  (JSON.parse(save.mock.calls.at(-1)![0]) as { tabs: SavedTab[] }).tabs;

describe("restoring the tabs from last time", { timeout: 15_000 }, () => {
  it("brings queries back right away and tables once their connection opens", async () => {
    const backend = await lastTime([scratch, customers, folio, report], customers.id);
    const { user } = renderApp(backend);

    await waitFor(() => expect(ids()).toEqual([scratch.id, customers.id, report.id]));
    // 上次停在表数据页:连上之后切回到它。
    expect(useTabs.getState().activeId).toBe(customers.id);
    // 两个查询都改过没保存:关标签页时照样会问。
    await waitFor(() => expect(useTabs.getState().unsaved).toEqual({ [scratch.id]: true, [report.id]: true }));
    expect(useUi.getState().notice?.message).toMatch(/「FOLIO 测试库」的 1 个标签页没有恢复（要输入密码）/);
    expect((await backend.listConnections()).find((c) => c.id === "mock-mysql")?.isOpen).toBe(true);
    const tabs = within(screen.getByRole("tablist"));
    expect(tabs.getByRole("tab", { name: /customers/ })).toHaveAttribute("aria-selected", "true");

    await user.click(tabs.getByRole("tab", { name: /查询 3/ }));
    expect((await findEditor("查询 3")).state.doc.toString()).toBe("SELECT 42;");
    await user.click(tabs.getByRole("tab", { name: /日报/ }));
    expect((await findEditor("日报")).state.doc.toString()).toBe("SELECT 2;");
  });

  it("records what is typed and which tabs are open, a moment after they change", async () => {
    const backend = createMockBackend();
    const save = vi.spyOn(backend, "saveWorkspace");
    const { user } = renderApp(backend);
    await screen.findByRole("heading", { name: /看清每一张表/ });
    const tree = within(screen.getByRole("tree", { name: "连接" }));
    const row = async (name: string) => (await tree.findByRole("treeitem", { name })).firstElementChild as HTMLElement;
    await user.dblClick(await row("开发"));
    await user.dblClick(await row("本地 MySQL"));
    await user.pointer({ keys: "[MouseRight]", target: await row("shop") });
    await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
    setSql(await findEditor("查询 1"), "SELECT 7;");

    await waitFor(() => expect(save.mock.calls.at(-1)?.[0]).toContain("SELECT 7;"), {
      timeout: SAVE_DELAY_MS * 6,
    });
    const recorded = JSON.parse(save.mock.calls.at(-1)![0]) as { activeId: string; tabs: SavedTab[] };
    expect(recorded.tabs).toEqual([
      expect.objectContaining({ kind: "query", title: "查询 1", sql: "SELECT 7;", baselineSql: "" }),
    ]);
    expect(recorded.activeId).toBe(recorded.tabs[0]!.id);

    await user.click(screen.getByRole("button", { name: "关闭 查询 1" }));
    await user.click(await screen.findByRole("button", { name: "放弃修改并关闭" }));
    await waitFor(() => expect(recordedTabs(save)).toEqual([]), {
      timeout: SAVE_DELAY_MS * 6,
    });
  });

  it("keeps tabs still waiting for their connection in the record", async () => {
    const base = await lastTime([scratch, customers], customers.id);
    let connect: () => void = () => undefined;
    const backend: Backend = {
      ...base,
      openConnection: (id, password) =>
        new Promise((resolve, reject) => {
          connect = () => void base.openConnection(id, password).then(resolve, reject);
        }),
    };
    const save = vi.spyOn(backend, "saveWorkspace");
    renderApp(backend);

    setSql(await findEditor("查询 3"), "SELECT 43;");
    await waitFor(() => expect(save.mock.calls.at(-1)?.[0]).toContain("SELECT 43;"), {
      timeout: SAVE_DELAY_MS * 6,
    });
    expect(recordedTabs(save).map((t) => t.id)).toEqual([scratch.id, customers.id]);
    expect(ids()).toEqual([scratch.id]);

    connect();
    await waitFor(() => expect(ids()).toEqual([scratch.id, customers.id]));
  });

  it("does not ask for an SSH passphrase or host key at startup, and says why the tabs stayed closed", async () => {
    for (const [code, reason] of [
      ["ssh_secret_required", "要输入 SSH 密码或私钥口令"],
      ["host_key_unknown", "第一次连这台 SSH 服务器，要先核对主机密钥"],
    ] as const) {
      const backend = await lastTime([scratch, customers], customers.id);
      vi.spyOn(backend, "openConnection").mockRejectedValue({ code, message: "后端原话", fields: [] });
      const view = renderApp(backend);
      await waitFor(() => expect(useUi.getState().notice?.message).toContain(`（${reason}）`));
      expect(ids()).toEqual([scratch.id]);
      expect(useUi.getState().passwordPrompt).toBeNull();
      view.unmount();
    }
  });

  it("starts empty when last time's record cannot be read", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "loadWorkspace").mockResolvedValue("{写到一半");
    renderApp(backend);
    expect(await screen.findByRole("heading", { name: /看清每一张表/ })).toBeInTheDocument();
    expect(useTabs.getState().tabs).toEqual([]);
  });
});
