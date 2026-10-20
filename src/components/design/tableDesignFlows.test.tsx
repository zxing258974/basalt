// 表结构设计器的主流程:从对象树打开、改字段后 ⌘S 预览并执行、新建表、校验失败、
// 部分生效、关闭前确认。用 mock 后端渲染整个应用。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { CommandError } from "../../lib/types";
import { closeTabGuarded } from "../../stores/discard";
import { useTabs } from "../../stores/tabs";
import { renderApp } from "../../test/renderApp";

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;

/** 展开「开发 → 本地 MySQL → shop → 表」。 */
async function openShop(backend?: Backend) {
  const context = renderApp(backend);
  const { user } = context;
  await user.dblClick(await findRow("开发"));
  await user.dblClick(await findRow("本地 MySQL"));
  await user.click(await findRow("shop"));
  await user.keyboard("{ArrowRight}");
  await user.dblClick(await findRow("表"));
  await findRow("customers");
  return context;
}

async function designCustomers(backend?: Backend) {
  const context = await openShop(backend);
  await context.user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
  await context.user.click(screen.getByRole("menuitem", { name: "设计表" }));
  await screen.findByRole("textbox", { name: "name：字段名" });
  return context;
}

const designer = () => screen.getByRole("region", { name: /的表结构$/ });

describe("designing an existing table", () => {
  it("opens from the table's menu, renames a column and saves after previewing the SQL", async () => {
    const { user, backend } = await designCustomers();
    expect(useTabs.getState().tabs.map((t) => [t.kind, t.title])).toEqual([["tableDesign", "customers"]]);
    expect(within(designer()).getByText("8 个字段 · 主键 id")).toBeInTheDocument();

    const name = screen.getByRole("textbox", { name: "name：字段名" });
    await user.clear(name);
    await user.type(name, "full_name");
    expect(screen.getByText(/1 处修改未保存/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /保存/ })).toHaveTextContent("1");

    await user.keyboard("{Meta>}s{/Meta}");
    const dialog = await screen.findByRole("dialog", { name: "保存表结构" });
    expect(await within(dialog).findByText(/RENAME COLUMN `name` TO `full_name`/)).toBeInTheDocument();
    expect(within(dialog).getByText(/逐条执行下面 1 条语句/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "执行" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getByText("已执行 1 条语句。")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "full_name：字段名" })).toBeInTheDocument();
    const saved = await backend.readTableDesign("mock-mysql", { database: "shop", schema: null, table: "customers" });
    expect(saved.columns[1]!.name).toBe("full_name");
    expect(useTabs.getState().unsaved).toEqual({});
  });

  it("renaming the table swaps the tab and refreshes the tree", async () => {
    const { user } = await designCustomers();
    const tableName = screen.getByRole("textbox", { name: "表名" });
    await user.clear(tableName);
    await user.type(tableName, "clients");
    expect(screen.getByText("改名")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /保存/ }));
    const dialog = await screen.findByRole("dialog", { name: "保存表结构" });
    await user.click(await within(dialog).findByRole("button", { name: "执行" }));

    await waitFor(() => expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["clients"]));
    expect(await findRow("clients")).toBeInTheDocument();
    expect(tree().queryByRole("treeitem", { name: "customers" })).not.toBeInTheDocument();
  });

  it("reverts edits and asks before closing a tab with unsaved changes", async () => {
    const { user } = await designCustomers();
    await user.click(screen.getByRole("checkbox", { name: "email：非空" }));
    expect(screen.getByRole("button", { name: "还原" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "还原" }));
    expect(screen.getByRole("checkbox", { name: "email：非空" })).not.toBeChecked();
    expect(screen.getByRole("button", { name: /保存/ })).toBeDisabled();

    await user.click(screen.getByRole("checkbox", { name: "email：非空" }));
    closeTabGuarded(useTabs.getState().activeId!);
    const prompt = await screen.findByRole("dialog", { name: "放弃未保存的修改？" });
    expect(within(prompt).getByText("有 1 张表的结构修改还没有保存，关闭后就没有了。")).toBeInTheDocument();
    await user.click(within(prompt).getByRole("button", { name: "放弃修改并关闭" }));
    expect(useTabs.getState().tabs).toEqual([]);
  });

  it("reports statements that already took effect and keeps the draft", async () => {
    const backend = createMockBackend();
    const partial: CommandError = {
      code: "partially_applied",
      message: "第 2 条语句执行失败：boom\n已经生效：第 1 条",
      fields: [],
    };
    vi.spyOn(backend, "applyTableDesign").mockRejectedValue(partial);
    const { user } = await designCustomers(backend);
    await user.click(screen.getByRole("checkbox", { name: "city：非空" }));
    await user.keyboard("{Meta>}s{/Meta}");
    const dialog = await screen.findByRole("dialog", { name: "保存表结构" });
    await user.click(await within(dialog).findByRole("button", { name: "执行" }));

    expect(await within(dialog).findByText("部分修改已经生效")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "执行" })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "关闭" }));
    expect(screen.getByRole("checkbox", { name: "city：非空" })).toBeChecked();
  });

  it("shows a plain failure inline and lets the user try again", async () => {
    const backend = createMockBackend();
    const failure: CommandError = { code: "database", message: "第 1 条语句执行失败，没有任何改动生效：x", fields: [] };
    vi.spyOn(backend, "applyTableDesign").mockRejectedValueOnce(failure);
    const { user } = await designCustomers(backend);
    await user.click(screen.getByRole("checkbox", { name: "city：非空" }));
    await user.keyboard("{Meta>}s{/Meta}");
    const dialog = await screen.findByRole("dialog", { name: "保存表结构" });
    await user.click(await within(dialog).findByRole("button", { name: "执行" }));
    expect(await within(dialog).findByText("执行失败")).toBeInTheDocument();
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });
});

describe("creating a table", () => {
  it("starts from an id column, refuses an empty name, then creates and reopens the new table", async () => {
    const { user } = await openShop();
    await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
    await user.click(screen.getByRole("menuitem", { name: "新建表" }));
    expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["新表"]);
    expect(screen.getByRole("textbox", { name: "表名" })).toHaveFocus();
    expect(screen.getByRole("textbox", { name: "id：字段名" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "创建" }));
    let dialog = await screen.findByRole("dialog", { name: "保存表结构" });
    expect(await within(dialog).findByText("请填写表名。")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "返回修改" }));

    await user.type(screen.getByRole("textbox", { name: "表名" }), "audit_log");
    await user.click(screen.getByRole("button", { name: "新增字段" }));
    await user.keyboard("message");
    await user.keyboard("{Meta>}s{/Meta}");
    dialog = await screen.findByRole("dialog", { name: "保存表结构" });
    expect(await within(dialog).findByText(/CREATE/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "执行" }));

    await waitFor(() => expect(useTabs.getState().tabs.map((t) => t.title)).toEqual(["audit_log"]));
    expect(await screen.findByRole("textbox", { name: "message：字段名" })).toBeInTheDocument();
    expect(await findRow("audit_log")).toBeInTheDocument();
  });

  it("offers 新建表 on PostgreSQL schemas but not on the database", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("FOLIO 测试库"));
    const prompt = await screen.findByRole("dialog", { name: "连接到「FOLIO 测试库」" });
    await user.type(within(prompt).getByLabelText("密码"), "folio");
    await user.click(within(prompt).getByRole("button", { name: "连接" }));

    await user.pointer({ keys: "[MouseRight]", target: await findRow("folio") });
    expect(screen.queryByRole("menuitem", { name: "新建表" })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(await findRow("folio"));
    await user.keyboard("{ArrowRight}");
    await user.pointer({ keys: "[MouseRight]", target: await findRow("public") });
    await user.click(screen.getByRole("menuitem", { name: "新建表" }));
    const tab = useTabs.getState().tabs[0]!;
    expect(tab).toMatchObject({ kind: "tableDesign", database: "folio", schema: "public", table: null });
  });
});
