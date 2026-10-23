// 对象树的交互:按需展开、键盘导航、右键菜单,以及各层的加载失败与重试。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import type { CommandError } from "../../lib/types";
import { useTabs } from "../../stores/tabs";
import { renderApp } from "../../test/renderApp";

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const item = (name: string) => tree().getByRole("treeitem", { name });
const findItem = (name: string) => tree().findByRole("treeitem", { name });
/** 鼠标事件挂在 treeitem 的第一行上,不在 treeitem 本身。 */
const rowOf = (name: string) => item(name).firstElementChild as HTMLElement;
const findRow = async (name: string) => (await findItem(name)).firstElementChild as HTMLElement;
const failure = (message: string): CommandError => ({ code: "query_failed", message, fields: [] });

describe("expanding", () => {
  it("puts ungrouped connections first, then folders", async () => {
    renderApp();
    await findItem("笔记");
    const names = tree()
      .getAllByRole("treeitem")
      .map((node) => node.getAttribute("aria-label"));
    expect(names).toEqual(["笔记", "开发"]);
    expect(within(item("开发")).getByText("3")).toBeInTheDocument();
  });

  it("opens a MySQL connection on expand and opens a database's objects on double click", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("本地 MySQL"));

    await user.dblClick(await findRow("shop"));
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["shop"]);
    expect(await screen.findByRole("heading", { name: "shop" })).toBeInTheDocument();

    const tables = await findItem("表");
    expect(within(tables).getByText("4")).toBeInTheDocument();
    await user.dblClick(rowOf("表"));
    expect(await findRow("orders")).toHaveAttribute("title", "订单主表");
  });

  it("asks for the password before expanding a PostgreSQL connection, then walks down to a schema", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("FOLIO 测试库"));

    const prompt = await screen.findByRole("dialog", { name: "连接到「FOLIO 测试库」" });
    await user.type(within(prompt).getByLabelText("密码"), "folio");
    await user.click(within(prompt).getByRole("button", { name: "连接" }));

    await user.dblClick(await findRow("folio"));
    await user.dblClick(await findRow("diku_mod_users"));
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["folio.diku_mod_users"]);
    expect(await findItem("视图")).toBeInTheDocument();
  });

  it("lists a Redis connection's databases with their key counts", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("本地 Redis"));

    const db0 = await findItem("db0（默认库）");
    expect(rowOf("db0（默认库）")).toHaveAttribute(
      "title",
      expect.stringMatching(/^\d+ 个键，其中 \d+ 个会过期。连接配置里的默认库$/),
    );
    expect(Number(within(db0).getByText(/^\d+$/).textContent)).toBeGreaterThan(40);
    expect(rowOf("db2")).toHaveAttribute("title", "0 个键");
    expect(tree().getAllByRole("treeitem", { name: /^db\d+/ })).toHaveLength(16);
  });

  it("opens Redis consoles from the menus and with ⌘T", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("本地 Redis"));
    await findItem("db0（默认库）");

    await user.pointer({ keys: "[MouseRight]", target: rowOf("本地 Redis") });
    await user.click(screen.getByRole("menuitem", { name: /^命令行/ }));
    await user.pointer({ keys: "[MouseRight]", target: rowOf("db3") });
    await user.click(screen.getByRole("menuitem", { name: /^在这个库打开命令行/ }));
    await user.click(rowOf("db5"));
    expect(screen.getByRole("button", { name: /^命令行/ })).toBeInTheDocument();
    await user.keyboard("{Meta>}t{/Meta}");

    await waitFor(() => expect(useTabs.getState().tabs).toHaveLength(3));
    const consoles = useTabs.getState().tabs.map((t) => (t.kind === "redisConsole" ? [t.title, t.db] : null));
    expect(consoles).toEqual([
      ["命令行", 0],
      ["命令行 2", 3],
      ["命令行 3", 5],
    ]);
    expect(await screen.findByRole("textbox", { name: "Redis 命令" })).toHaveFocus();
  });

  it("opens the object list from an empty group", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("本地 MySQL"));
    // mysql 库只有一张表,没有视图:「视图」分组不能展开,双击直接打开对象列表。
    const database = await findItem("mysql");
    await user.click(within(database).getByText("mysql"));
    await user.keyboard("{ArrowRight}");
    const views = await within(database).findByRole("treeitem", { name: "视图" });
    expect(views).not.toHaveAttribute("aria-expanded");
    await user.dblClick(views.firstElementChild as HTMLElement);
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["mysql"]);
  });
});

describe("keyboard", () => {
  it("moves, expands and collapses with the arrow keys", async () => {
    const { user } = renderApp();
    await user.click(within(await findItem("笔记")).getByText("笔记"));
    expect(item("笔记")).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(item("开发")).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(item("开发")).toHaveAttribute("aria-expanded", "true");
    await user.keyboard("{ArrowRight}");
    expect(item("本地 MySQL")).toHaveFocus();

    await user.keyboard("{End}");
    expect(item("本地 Redis")).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(item("开发")).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(item("开发")).toHaveAttribute("aria-expanded", "false");
    await user.keyboard("{Home}");
    expect(item("笔记")).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(item("笔记")).toHaveFocus();
  });

  it("opens the selected database with Enter", async () => {
    const { user } = renderApp();
    await user.click(within(await findItem("笔记")).getByText("笔记"));
    await user.keyboard("{Enter}");
    await user.click(within(await findItem("main")).getByText("main"));
    await user.keyboard("{Enter}");
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["main"]);
  });
});

describe("context menu", () => {
  async function openMenu(name: string) {
    const context = renderApp();
    await findItem("笔记");
    await context.user.pointer({ keys: "[MouseRight]", target: rowOf(name) });
    return { ...context, menu: within(screen.getByRole("menu")) };
  }

  it("opens and closes a connection", async () => {
    const { user, menu } = await openMenu("笔记");
    expect(menu.getByRole("menuitem", { name: "刷新" })).toBeDisabled();
    await user.click(menu.getByRole("menuitem", { name: "打开连接" }));
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await findItem("main");

    await user.pointer({ keys: "[MouseRight]", target: rowOf("笔记") });
    await user.click(screen.getByRole("menuitem", { name: "刷新" }));
    await user.pointer({ keys: "[MouseRight]", target: rowOf("笔记") });
    await user.click(screen.getByRole("menuitem", { name: "关闭连接" }));
    await waitFor(() => expect(tree().queryByRole("treeitem", { name: "main" })).not.toBeInTheDocument());
  });

  it("duplicates a connection", async () => {
    const { user, menu } = await openMenu("笔记");
    await user.click(menu.getByRole("menuitem", { name: "复制连接" }));
    expect(await findItem("笔记 副本")).toBeInTheDocument();
  });

  it("edits a connection", async () => {
    const { user, menu } = await openMenu("笔记");
    await user.click(menu.getByRole("menuitem", { name: "编辑连接…" }));
    const dialog = screen.getByRole("dialog", { name: "编辑连接" });
    expect(within(dialog).getByLabelText("连接名")).toHaveValue("笔记");
  });

  it("deletes a connection after confirming", async () => {
    const { user, menu } = await openMenu("笔记");
    await user.click(menu.getByRole("menuitem", { name: "删除连接…" }));
    const dialog = screen.getByRole("dialog", { name: "删除连接「笔记」?" });
    await user.click(within(dialog).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(tree().queryByRole("treeitem", { name: "笔记" })).not.toBeInTheDocument());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("says so when the password could not be removed from the keychain", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "deleteConnection").mockResolvedValue({ passwordLeft: true });
    const context = renderApp(backend);
    await findItem("笔记");
    await context.user.pointer({ keys: "[MouseRight]", target: rowOf("笔记") });
    await context.user.click(screen.getByRole("menuitem", { name: "删除连接…" }));
    await context.user.click(screen.getByRole("button", { name: "删除" }));
    const notice = await within(screen.getByRole("contentinfo")).findByRole("alert");
    expect(notice).toHaveTextContent("密码还留在钥匙串里");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("keeps the delete dialog open and shows why deleting failed", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "deleteConnection").mockRejectedValue(failure("配置文件被占用"));
    const context = renderApp(backend);
    await findItem("笔记");
    await context.user.pointer({ keys: "[MouseRight]", target: rowOf("笔记") });
    await context.user.click(screen.getByRole("menuitem", { name: "删除连接…" }));
    await context.user.click(screen.getByRole("button", { name: "删除" }));
    expect(await screen.findByText("配置文件被占用")).toBeInTheDocument();
    await context.user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("opens the object list and refreshes from a database's menu", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("笔记"));
    await user.pointer({ keys: "[MouseRight]", target: await findRow("main") });
    await user.click(screen.getByRole("menuitem", { name: "打开对象列表" }));
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["main"]);

    await user.pointer({ keys: "[MouseRight]", target: rowOf("main") });
    await user.click(screen.getByRole("menuitem", { name: "刷新" }));
    expect(await findItem("main")).toBeInTheDocument();
  });

  it("opens a schema's object list from its menu", async () => {
    const backend = createMockBackend();
    await backend.openConnection("mock-pg", "folio");
    const { user } = renderApp(backend);
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("FOLIO 测试库"));
    await user.dblClick(await findRow("folio"));
    await user.pointer({ keys: "[MouseRight]", target: await findRow("public") });
    await user.click(screen.getByRole("menuitem", { name: "打开对象列表" }));
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["folio.public"]);
  });
});

describe("failures", () => {
  it("offers a retry when the connection list cannot be read", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "listConnections").mockRejectedValueOnce(failure("配置文件损坏"));
    const { user } = renderApp(backend);
    expect(await tree().findByText("配置文件损坏")).toBeInTheDocument();
    await user.click(tree().getByRole("button", { name: "重试" }));
    expect(await findItem("笔记")).toBeInTheDocument();
  });

  it("shows why a connection failed to open under its node and retries from there", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "openConnection").mockRejectedValueOnce(failure("文件不存在"));
    const { user } = renderApp(backend);
    await user.dblClick(await findRow("笔记"));
    expect(await tree().findByText("文件不存在")).toBeInTheDocument();
    await user.click(tree().getByRole("button", { name: "重试" }));
    expect(await findItem("main")).toBeInTheDocument();
  });

  it("retries a database list that failed to load", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "listDatabases").mockRejectedValueOnce(failure("权限不足"));
    const { user } = renderApp(backend);
    await user.dblClick(await findRow("笔记"));
    expect(await tree().findByText("权限不足")).toBeInTheDocument();
    await user.click(tree().getByRole("button", { name: "重试" }));
    expect(await findItem("main")).toBeInTheDocument();
  });

  it("says so when a server has no databases", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "listDatabases").mockResolvedValue([]);
    const { user } = renderApp(backend);
    await user.dblClick(await findRow("笔记"));
    expect(await tree().findByText("没有可访问的数据库")).toBeInTheDocument();
  });
});
