// 清空、截断、重命名表与复制表名(mock 后端):两种清空的区别、开着的数据页重读、外键拒绝时的提示、
// 改名后标签页和选中项跟着换、剪贴板。
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import type { ReferencingKey, TablePage } from "../../lib/types";
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
    expect(dialog.getByText("表里的全部行（约 18,204 行，估算）都会删掉，不能撤销。")).toBeInTheDocument();
    expect(dialog.getByText("表结构、索引都留着；开着的数据页会重新读取。")).toBeInTheDocument();
    await waitFor(() => expect(read).toHaveBeenCalledTimes(1));

    await user.click(dialog.getByRole("button", { name: "清空" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(clear).toHaveBeenCalledWith("mock-mysql", CUSTOMERS, "delete");
    expect(notice()).toHaveTextContent(/^已清空表「customers」，删除了 \d+ 行。$/);
    await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    const page = (await read.mock.results[1]!.value) as TablePage;
    expect(page.rows).toEqual([]);
  });

  it("truncates without a row count, switches wording and shows 0 rows afterwards", async () => {
    const backend = createMockBackend();
    const { user } = await openTableMenu("截断表…", { ...MYSQL_CUSTOMERS, table: "orders" }, backend);
    const dialog = within(screen.getByRole("dialog", { name: "截断表「orders」？" }));
    await user.click(dialog.getByRole("radio", { name: /^清空/ }));
    expect(screen.getByRole("dialog", { name: "清空表「orders」？" })).toBeInTheDocument();
    await user.click(dialog.getByRole("radio", { name: /^截断/ }));
    await user.click(dialog.getByRole("button", { name: "截断" }));
    await waitFor(() => expect(notice()).toHaveTextContent("已截断表「orders」。"));
    // 和真库一样:对象列表里的行数、之后导出报的行数都是 0。
    const orders = (await backend.listTables("mock-mysql", "shop")).find((t) => t.name === "orders");
    expect(orders?.rows).toBe(0);
    const exported = await backend.exportData("job", {
      connectionId: "mock-mysql",
      source: { kind: "table", table: { database: "shop", schema: null, table: "orders" }, filter: null, sort: null },
      options: { format: { kind: "xlsx" }, binary: "hex" },
      path: "/tmp/orders.xlsx",
    });
    expect(exported.rows).toBe(0);
  });

  it("explains a foreign key refusal and clears the error when switching to delete", async () => {
    // mock 和真库一样:customers 被 orders 引用,截断被拒绝。
    const { user } = await openTableMenu("截断表…");
    const dialog = within(screen.getByRole("dialog", { name: "截断表「customers」？" }));
    // mock 里 orders 以 ON DELETE CASCADE 引用 customers。
    expect(await dialog.findByLabelText("外键")).toHaveTextContent("被 orders 的外键引用：截断会被数据库拒绝。");
    await user.click(dialog.getByRole("button", { name: "截断" }));
    const alert = await dialog.findByRole("alert");
    expect(alert).toHaveTextContent("foreign key constraint");
    expect(alert).toHaveTextContent("被别的表的外键引用时不能截断。改用「清空」会按外键连带改动 orders 里的行。");
    await user.click(dialog.getByRole("radio", { name: /^清空/ }));
    expect(dialog.queryByRole("alert")).not.toBeInTheDocument();
    expect(dialog.getByLabelText("外键")).toHaveTextContent("会按外键连带改动别的表：orders 里引用它的行会被删掉。");
    await user.click(dialog.getByRole("button", { name: "清空" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("notes tables that would block a clear and says when it cannot tell", async () => {
    const backend = createMockBackend();
    const { user } = await openTableMenu("清空表…", { ...MYSQL_CUSTOMERS, table: "products" }, backend);
    const dialog = within(screen.getByRole("dialog", { name: "清空表「products」？" }));
    expect(await dialog.findByText("order_items 也引用这张表：那边还有引用它的行时，会删不掉。")).toBeInTheDocument();
    expect(dialog.queryByLabelText("外键")).not.toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "取消" }));

    vi.spyOn(backend, "tableReferences").mockRejectedValue({ code: "database", message: "denied", fields: [] });
    await user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
    await user.click(screen.getByRole("menuitem", { name: "清空表…" }));
    expect(
      await within(screen.getByRole("dialog")).findByText("没能查到别的表上引用它的外键：denied"),
    ).toBeInTheDocument();
  });

  it("asks before throwing away uncommitted edits on the data page, like closing it would", async () => {
    const backend = createMockBackend();
    const clear = vi.spyOn(backend, "clearTable");
    const { user } = renderApp(backend);
    const tab = tableDataTab("mock-mysql", "shop", null, "customers");
    const connection = (await backend.listConnections()).find((c) => c.id === "mock-mysql")!;
    // 分开 act:数据页挂上时会先把自己的未提交行数(0)报上去。
    act(() => useTabs.getState().open(tab));
    act(() => useTabs.getState().setPending(tab.id, 3));
    act(() => useTableOps.getState().open({ kind: "clear", mode: "delete", rows: null, connection, table: CUSTOMERS }));
    const dialog = within(screen.getByRole("dialog", { name: "清空表「customers」？" }));
    expect(dialog.getByText(/^表里的全部行都会删掉/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "清空" }));
    const ask = within(await screen.findByRole("dialog", { name: "放弃未提交的修改？" }));
    expect(ask.getByText("有 3 行修改还没有提交，清空后就没有了。")).toBeInTheDocument();
    await user.click(ask.getByRole("button", { name: "取消" }));
    expect(clear).not.toHaveBeenCalled();

    await user.click(dialog.getByRole("button", { name: "清空" }));
    await user.click(
      within(await screen.findByRole("dialog", { name: "放弃未提交的修改？" })).getByRole("button", {
        name: "放弃修改并清空",
      }),
    );
    await waitFor(() => expect(clear).toHaveBeenCalledTimes(1));
  });

  it("also asks about uncommitted edits in tables the clear would cascade into", async () => {
    const backend = createMockBackend();
    const clear = vi.spyOn(backend, "clearTable");
    await backend.openConnection("mock-mysql");
    const { user } = renderApp(backend);
    const orders = tableDataTab("mock-mysql", "shop", null, "orders");
    const connection = (await backend.listConnections()).find((c) => c.id === "mock-mysql")!;
    act(() => useTabs.getState().open(orders));
    act(() => useTabs.getState().setPending(orders.id, 2));
    // mock 里 orders 以 ON DELETE CASCADE 引用 customers:清空 customers 会连带删 orders 的行。
    act(() => useTableOps.getState().open({ kind: "clear", mode: "delete", rows: null, connection, table: CUSTOMERS }));
    const dialog = within(screen.getByRole("dialog", { name: "清空表「customers」？" }));
    expect(await dialog.findByLabelText("外键")).toHaveTextContent("orders 里引用它的行会被删掉");
    expect(dialog.getByText("表结构、索引都留着；开着的数据页会重新读取。")).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "清空" }));
    const ask = within(await screen.findByRole("dialog", { name: "放弃未提交的修改？" }));
    expect(ask.getByText("有 2 行修改还没有提交，清空后就没有了。")).toBeInTheDocument();
    await user.click(ask.getByRole("button", { name: "放弃修改并清空" }));
    await waitFor(() => expect(clear).toHaveBeenCalledTimes(1));
  });

  it("cannot be confirmed before it knows which tables would be affected", async () => {
    const backend = createMockBackend();
    let answer: (keys: ReferencingKey[]) => void = () => undefined;
    vi.spyOn(backend, "tableReferences").mockImplementationOnce(() => new Promise((resolve) => (answer = resolve)));
    const { user } = await openTableMenu("清空表…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "清空表「customers」？" }));
    expect(dialog.getByRole("button", { name: "清空" })).toBeDisabled();
    expect(dialog.getByText("正在查别的表上引用它的外键…")).toBeInTheDocument();
    act(() => answer([{ table: { ...CUSTOMERS, table: "orders" }, name: "fk_orders_customer", onDelete: "cascade" }]));
    await waitFor(() => expect(dialog.getByRole("button", { name: "清空" })).toBeEnabled());
    expect(dialog.getByLabelText("外键")).toHaveTextContent("orders 里引用它的行会被删掉");
    expect(dialog.queryByText("正在查别的表上引用它的外键…")).not.toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "取消" }));
  });

  it("says a SQLite truncate blocked by foreign keys needs those rows handled first", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "clearTable").mockRejectedValueOnce({
      code: "database",
      message: "FOREIGN KEY constraint failed",
      fields: [],
    });
    const { user } = await openTableMenu("截断表…", SQLITE_NOTES, backend);
    const dialog = within(screen.getByRole("dialog", { name: "截断表「notes」？" }));
    await user.click(dialog.getByRole("button", { name: "截断" }));
    // SQLite 的截断就是 DELETE:换成「清空」也一样会失败,不能这么建议。
    const alert = await dialog.findByRole("alert");
    expect(alert).toHaveTextContent("别的表里还有引用它的行，外键不让删：先删掉或改掉那些行再试。");
    expect(alert).not.toHaveTextContent("改用「清空」");
  });

  it("keeps the hint when a refused truncate finishes in the background", async () => {
    const backend = createMockBackend();
    let fail: (error: unknown) => void = () => undefined;
    vi.spyOn(backend, "clearTable").mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)));
    const { user } = await openTableMenu("截断表…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "截断表「customers」？" }));
    await dialog.findByLabelText("外键");
    await user.click(dialog.getByRole("button", { name: "截断" }));
    await user.click(dialog.getByRole("button", { name: "在后台继续" }));
    act(() =>
      fail({ code: "database", message: "Cannot truncate a table referenced in a foreign key constraint", fields: [] }),
    );
    await waitFor(() =>
      expect(notice()).toHaveTextContent(/截断表「customers」失败：.*改用「清空」会按外键连带改动 orders 里的行/),
    );
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

  it("asks before replacing tabs that hold unsaved work", async () => {
    const backend = createMockBackend();
    const rename = vi.spyOn(backend, "renameTable");
    const { user } = renderApp(backend);
    const design = tableDesignTab("mock-mysql", "shop", null, "customers");
    const connection = (await backend.listConnections()).find((c) => c.id === "mock-mysql")!;
    act(() => useTabs.getState().open(design));
    act(() => useTabs.getState().setUnsaved(design.id, true));
    act(() => useTableOps.getState().open({ kind: "rename", connection, table: CUSTOMERS }));
    const dialog = within(screen.getByRole("dialog", { name: "重命名表" }));
    const name = dialog.getByLabelText("新表名");
    await user.clear(name);
    await user.type(name, "customers_old");
    await user.click(dialog.getByRole("button", { name: "重命名" }));
    const ask = within(await screen.findByRole("dialog", { name: "放弃未保存的修改？" }));
    expect(ask.getByText("有 1 张表的结构修改还没有保存，改名后就没有了。")).toBeInTheDocument();
    await user.click(ask.getByRole("button", { name: "放弃修改并改名" }));
    await waitFor(() => expect(rename).toHaveBeenCalledWith("mock-mysql", CUSTOMERS, "customers_old"));
  });

  it("can keep renaming in the background and still follows the new name", async () => {
    const backend = createMockBackend();
    let finish: () => void = () => undefined;
    const real = backend.renameTable.bind(backend);
    vi.spyOn(backend, "renameTable").mockImplementationOnce(
      (id, table, name) => new Promise((resolve) => (finish = () => resolve(real(id, table, name)))),
    );
    const { user } = await openTableMenu("重命名…", MYSQL_CUSTOMERS, backend, () =>
      useTabs.getState().open(tableDataTab("mock-mysql", "shop", null, "customers")),
    );
    const dialog = within(screen.getByRole("dialog", { name: "重命名表" }));
    const name = dialog.getByLabelText("新表名");
    await user.clear(name);
    await user.type(name, "clients{Enter}");
    // 表被别的会话占着、正在等锁:先关掉对话框回去提交那边的事务。
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    act(() => finish());
    await waitFor(() => expect(notice()).toHaveTextContent("已把「customers」改名为「clients」。"));
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["clients"]);
  });

  it("reports a busy table when renaming in the background fails", async () => {
    const backend = createMockBackend();
    let fail: (error: unknown) => void = () => undefined;
    vi.spyOn(backend, "renameTable").mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)));
    const { user } = await openTableMenu("重命名…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "重命名表" }));
    await user.type(dialog.getByLabelText("新表名"), "2");
    await user.click(dialog.getByRole("button", { name: "重命名" }));
    await user.click(dialog.getByRole("button", { name: "在后台继续" }));
    act(() => fail({ code: "database", message: "这张表正被别的会话占用", fields: [] }));
    await waitFor(() => expect(notice()).toHaveTextContent("把「customers」改名失败：这张表正被别的会话占用"));
  });

  it("forgets stale caches under the old and the new name", async () => {
    const backend = createMockBackend();
    const { user, queryClient } = await openTableMenu("重命名…", { ...MYSQL_CUSTOMERS, table: "products" }, backend);
    // 假装 orders 这个名字以前的表在内存里还留着一页数据、一份表结构。
    const stale = { database: "shop", schema: null, table: "orders" };
    queryClient.setQueryData(["tableData", "mock-mysql", "shop", "", "orders", { offset: 0 }], { rows: ["旧"] });
    queryClient.setQueryData(["tableDesign", "mock-mysql", "shop", "", "orders"], { columns: ["旧"] });
    await backend.dropTable("mock-mysql", stale);
    const dialog = within(screen.getByRole("dialog", { name: "重命名表" }));
    const name = dialog.getByLabelText("新表名");
    await user.clear(name);
    await user.type(name, "orders{Enter}");
    await waitFor(() => expect(notice()).toHaveTextContent("已把「products」改名为「orders」。"));
    const cached = (key: unknown[]) => queryClient.getQueryCache().findAll({ queryKey: key });
    expect(cached(["tableData", "mock-mysql", "shop", "", "orders"])).toEqual([]);
    expect(cached(["tableDesign", "mock-mysql", "shop", "", "orders"])).toEqual([]);
    // 改名后的表沿用原来的行数、引擎(和真库一样)。
    const renamed = (await backend.listTables("mock-mysql", "shop")).find((t) => t.name === "orders");
    expect(renamed).toMatchObject({ engine: "InnoDB", rows: 1_204, comment: "商品" });
  });

  it("refuses PostgreSQL names longer than 63 bytes", async () => {
    const backend = createMockBackend();
    await backend.openConnection("mock-pg", "secret");
    const users = { database: "folio", schema: "diku_mod_users", table: "users" };
    await expect(backend.renameTable("mock-pg", users, "表".repeat(22))).rejects.toMatchObject({
      code: "invalid_data",
      message: expect.stringContaining("66 字节") as string,
    });
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
