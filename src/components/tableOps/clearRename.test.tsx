// 清空、截断、重命名表与复制表名(mock 后端):两种清空的区别、开着的数据页重读、外键拒绝时的提示、
// 改名后标签页和选中项跟着换、剪贴板。
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import type { TablePage } from "../../lib/types";
import { useTableOps } from "../../stores/tableOps";
import { tableDataTab, tableDesignTab, useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { renderApp } from "../../test/renderApp";
import { MYSQL_CUSTOMERS, SQLITE_NOTES, findRow, openTableMenu, tree } from "../../test/tableMenu";
import { tableKey } from "../sidebar/treeKeys";

/** 状态栏里的提示。开着设计器时页面上有两个 footer,按类名找状态栏。 */
const notice = () => within(document.querySelector<HTMLElement>(".statusbar")!).getByRole("alert");
const CUSTOMERS = { database: "shop", schema: null, table: "customers" };

describe("clear and truncate", { timeout: 15_000 }, () => {
  it("empties the table, keeps both choices explained and reloads the open data page", async () => {
    const backend = createMockBackend();
    const read = vi.spyOn(backend, "readTablePage");
    const clear = vi.spyOn(backend, "clearTable");
    const { user } = await openTableMenu("清空表…", MYSQL_CUSTOMERS, backend, () =>
      useTabs.getState().open(tableDataTab("mock-mysql", "shop", null, "customers")),
    );
    const dialog = within(screen.getByRole("dialog", { name: "清空表「customers」？" }));
    expect(dialog.getByRole("radio", { name: /^清空DELETE FROM/ })).toHaveAttribute("aria-checked", "true");
    expect(dialog.getByRole("radio", { name: /^截断TRUNCATE.*不能回滚.*外键/ })).toBeInTheDocument();
    expect(dialog.getByText(/不能撤销；表结构、索引都留着。开着的数据页会重新读取/)).toBeInTheDocument();
    await waitFor(() => expect(read).toHaveBeenCalledTimes(1));

    await user.click(dialog.getByRole("button", { name: "清空" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(clear).toHaveBeenCalledWith("mock-mysql", CUSTOMERS, "delete");
    expect(notice()).toHaveTextContent(/^已清空表「customers」，删除了 \d+ 行。$/);
    await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    const page = (await read.mock.results[1]!.value) as TablePage;
    expect(page.rows).toEqual([]);
  });

  it("truncates without a row count and switching the choice changes the wording", async () => {
    const { user } = await openTableMenu("截断表…");
    const dialog = within(screen.getByRole("dialog", { name: "截断表「customers」？" }));
    await user.click(dialog.getByRole("radio", { name: /^清空/ }));
    expect(screen.getByRole("dialog", { name: "清空表「customers」？" })).toBeInTheDocument();
    await user.click(dialog.getByRole("radio", { name: /^截断/ }));
    await user.click(dialog.getByRole("button", { name: "截断" }));
    await waitFor(() => expect(notice()).toHaveTextContent("已截断表「customers」。"));
  });

  it("explains a foreign key refusal and clears the error when switching to delete", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "clearTable").mockRejectedValueOnce({
      code: "database",
      message: "Cannot truncate a table referenced in a foreign key constraint",
      fields: [],
    });
    const { user } = await openTableMenu("截断表…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "截断表「customers」？" }));
    await user.click(dialog.getByRole("button", { name: "截断" }));
    const alert = await dialog.findByRole("alert");
    expect(alert).toHaveTextContent("foreign key constraint");
    expect(alert).toHaveTextContent("被别的表的外键引用时不能截断，可以改用「清空」。");
    await user.click(dialog.getByRole("radio", { name: /^清空/ }));
    expect(dialog.queryByRole("alert")).not.toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "清空" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("warns that uncommitted edits on the data page will be lost", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "clearTable").mockRejectedValueOnce({ code: "database", message: "busy", fields: [] });
    const { user } = renderApp(backend);
    const tab = tableDataTab("mock-mysql", "shop", null, "customers");
    const connection = (await backend.listConnections()).find((c) => c.id === "mock-mysql")!;
    // 分开 act:数据页挂上时会先把自己的未提交行数(0)报上去。
    act(() => useTabs.getState().open(tab));
    act(() => useTabs.getState().setPending(tab.id, 3));
    act(() => useTableOps.getState().open({ kind: "clear", mode: "delete", connection, table: CUSTOMERS }));
    const dialog = within(screen.getByRole("dialog", { name: "清空表「customers」？" }));
    expect(dialog.getByText("这张表还有 3 行修改没有提交，清空后就没有了。")).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "清空" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("busy");
    expect(dialog.getByRole("alert")).not.toHaveTextContent("外键");
  });

  it("can keep running in the background and reports the outcome in the status bar", async () => {
    const backend = createMockBackend();
    let finish: (rows: number) => void = () => undefined;
    let fail: (error: unknown) => void = () => undefined;
    vi.spyOn(backend, "clearTable")
      .mockImplementationOnce(() => new Promise((resolve) => (finish = (rows) => resolve({ rows }))))
      .mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)));
    const { user } = await openTableMenu("清空表…", MYSQL_CUSTOMERS, backend);
    let dialog = within(screen.getByRole("dialog", { name: "清空表「customers」？" }));
    await user.click(dialog.getByRole("button", { name: "清空" }));
    await user.click(dialog.getByRole("button", { name: "在后台继续" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    act(() => finish(18_204));
    await waitFor(() => expect(notice()).toHaveTextContent("已清空表「customers」，删除了 18,204 行。"));

    await user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
    await user.click(screen.getByRole("menuitem", { name: "截断表…" }));
    dialog = within(screen.getByRole("dialog", { name: "截断表「customers」？" }));
    await user.click(dialog.getByRole("button", { name: "截断" }));
    // jsdom 不会把 Esc 变成 <dialog> 的 cancel 事件:直接派发。
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    act(() => fail({ code: "database", message: "Lock wait timeout exceeded", fields: [] }));
    await waitFor(() => expect(notice()).toHaveTextContent("截断表「customers」失败：Lock wait timeout exceeded"));
  });

  it("describes SQLite's delete-based truncation and reports its row count", async () => {
    const { user } = await openTableMenu("截断表…", SQLITE_NOTES);
    const dialog = within(screen.getByRole("dialog", { name: "截断表「notes」？" }));
    expect(dialog.getByRole("radio", { name: /SQLite 没有 TRUNCATE/ })).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "截断" }));
    await waitFor(() => expect(notice()).toHaveTextContent(/^已截断表「notes」，删除了 \d+ 行。$/));
  });
});

describe("rename", { timeout: 15_000 }, () => {
  it("renames the table and moves its tabs and the selection to the new name", async () => {
    const backend = createMockBackend();
    const rename = vi.spyOn(backend, "renameTable");
    const { user, queryClient } = await openTableMenu("重命名…", MYSQL_CUSTOMERS, backend, () => {
      const { open } = useTabs.getState();
      open(tableDesignTab("mock-mysql", "shop", null, "customers"));
      open(tableDataTab("mock-mysql", "shop", null, "customers"));
      open(tableDataTab("mock-mysql", "shop", null, "orders"));
    });
    act(() => useUi.getState().select(tableKey("mock-mysql", "shop", null, "customers")));
    const dialog = within(screen.getByRole("dialog", { name: "重命名表" }));
    expect(dialog.getByText(/视图、存储过程里写的旧表名不会/)).toBeInTheDocument();
    const name = dialog.getByLabelText("新表名");
    expect(name).toHaveValue("customers");
    expect(dialog.getByRole("button", { name: "重命名" })).toBeDisabled();
    await user.clear(name);
    await user.type(name, " clients {Enter}");

    expect(await tree().findByRole("treeitem", { name: "clients" })).toBeInTheDocument();
    expect(tree().queryByRole("treeitem", { name: "customers" })).not.toBeInTheDocument();
    expect(rename).toHaveBeenCalledWith("mock-mysql", CUSTOMERS, "clients");
    const { tabs, activeId } = useTabs.getState();
    expect(tabs.map((t) => [t.id, t.title])).toEqual([
      [tableDesignTab("mock-mysql", "shop", null, "clients").id, "clients"],
      [tableDataTab("mock-mysql", "shop", null, "clients").id, "clients"],
      [tableDataTab("mock-mysql", "shop", null, "orders").id, "orders"],
    ]);
    expect(activeId).toBe(tableDataTab("mock-mysql", "shop", null, "orders").id);
    expect(useUi.getState().selectedKey).toBe(tableKey("mock-mysql", "shop", null, "clients"));
    expect(notice()).toHaveTextContent("已把「customers」改名为「clients」。");
    expect(
      queryClient.getQueryCache().findAll({ queryKey: ["tableData", "mock-mysql", "shop", "", "customers"] }),
    ).toEqual([]);
  });

  it("keeps the dialog open when the new name is taken", async () => {
    const { user } = await openTableMenu("重命名…");
    const dialog = within(screen.getByRole("dialog", { name: "重命名表" }));
    const name = dialog.getByLabelText("新表名");
    await user.clear(name);
    await user.type(name, "orders");
    await user.click(dialog.getByRole("button", { name: "重命名" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("already exists");
    expect(tree().getByRole("treeitem", { name: "customers" })).toBeInTheDocument();
  });

  it("warns about unsaved work in the tabs it will replace", async () => {
    const backend = createMockBackend();
    renderApp(backend);
    const design = tableDesignTab("mock-mysql", "shop", null, "customers");
    const connection = (await backend.listConnections()).find((c) => c.id === "mock-mysql")!;
    act(() => useTabs.getState().open(design));
    act(() => useTabs.getState().setUnsaved(design.id, true));
    act(() => useTableOps.getState().open({ kind: "rename", connection, table: CUSTOMERS }));
    const dialog = within(screen.getByRole("dialog", { name: "重命名表" }));
    expect(dialog.getByText("这张表还有设计器里的结构修改没有保存，改名后就没有了。")).toBeInTheDocument();
  });
});

describe("copy name", { timeout: 15_000 }, () => {
  it("copies the bare table name and says so", async () => {
    const write = vi.fn(() => Promise.resolve());
    const app = renderApp();
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    const { user } = app;
    for (const name of MYSQL_CUSTOMERS.path) await user.dblClick(await findRow(name));
    await user.click(await findRow("shop"));
    await user.keyboard("{ArrowRight}");
    await user.dblClick(await findRow("视图"));
    await user.pointer({ keys: "[MouseRight]", target: await findRow("v_daily_sales") });
    await user.click(screen.getByRole("menuitem", { name: "复制视图名" }));
    expect(write).toHaveBeenCalledWith("v_daily_sales");
    await waitFor(() => expect(notice()).toHaveTextContent("已复制「v_daily_sales」。"));
  });

  it("reports when the clipboard refuses or is missing", async () => {
    const write = vi.fn(() => Promise.reject(new Error("denied")));
    const app = await openTableMenu("重命名…");
    await app.user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "取消" }));
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    await app.user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
    await app.user.click(screen.getByRole("menuitem", { name: "复制表名" }));
    await waitFor(() => expect(notice()).toHaveTextContent("没能复制：Error: denied"));

    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    await app.user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
    await app.user.click(screen.getByRole("menuitem", { name: "复制表名" }));
    await waitFor(() => expect(notice()).toHaveTextContent("没能复制：拿不到剪贴板。"));
  });
});
