// 查询结果的流程(mock 后端):往下滚分批取行、单表结果直接改并提交、有修改时锁住执行和换标签。
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import { useTabs } from "../../stores/tabs";
import { findEditor, installLayoutStubs, setSql } from "../../test/editor";
import { renderApp } from "../../test/renderApp";

installLayoutStubs();

const FLOW_TIMEOUT = 15_000;

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;
const header = () => within(screen.getByRole("region", { name: "查询 1（SQL 查询）" }));

/** 打开本地 MySQL,在 shop 上新建查询,执行(全部)`sql`,等结果表格出来。 */
async function runInShop(sql: string) {
  const backend = createMockBackend();
  const app = renderApp(backend);
  const { user } = app;
  await user.dblClick(await findRow("开发"));
  await user.dblClick(await findRow("本地 MySQL"));
  await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
  await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
  const view = await findEditor("查询 1");
  setSql(view, sql);
  await user.click(header().getByRole("button", { name: "全部执行" }));
  const grid = await screen.findByRole("grid", { name: "查询 1 的结果 1" });
  await waitFor(() => expect(within(grid).getAllByRole("row").length).toBeGreaterThan(1));
  return { ...app, backend, view, grid };
}

/** jsdom 里改 scrollTop 不会触发 scroll 事件,虚拟滚动看不到:手动补一个。 */
function scrollTo(grid: HTMLElement, row: number) {
  const scroller = grid.parentElement!;
  scroller.scrollTop = row * 26;
  fireEvent.scroll(scroller);
}

/** 第 row 个数据行(从 0 数)的第 col 个格子。 */
function cell(grid: HTMLElement, row: number, col: number): HTMLElement {
  const dataRow = within(grid)
    .getAllByRole("row")
    .find((r) => r.getAttribute("aria-rowindex") === String(row + 2));
  return within(dataRow!).getAllByRole("gridcell")[col]!;
}

describe("query results", { timeout: FLOW_TIMEOUT }, () => {
  it("shows every row: more are fetched when the end of what's loaded comes into view", async () => {
    const { grid } = await runInShop("SELECT * FROM orders");
    expect(screen.getByText("已显示 1,000 / 共 1,500 行")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /结果 1/ })).toHaveTextContent("1500");

    scrollTo(grid, 980);
    expect(await screen.findByText("1,500 行")).toBeInTheDocument();
    scrollTo(grid, 1490);
    await waitFor(() => expect(cell(grid, 1499, 0)).toBeInTheDocument());
  });

  it("shows the selected cell of the result in the inspector's value tab", async () => {
    const { grid, user } = await runInShop("SELECT id, shipping FROM orders");
    await user.pointer({ keys: "[MouseRight]", target: cell(grid, 1, 1) });
    await user.click(screen.getByRole("menuitem", { name: "在右侧查看完整内容" }));
    const panel = within(screen.getByRole("complementary", { name: "信息" }));
    expect(panel.getByRole("heading", { name: "shipping" })).toBeInTheDocument();
    expect(panel.getByText(/查询 1 的结果 1 · 第 2 行$/)).toBeInTheDocument();
    expect(panel.getByLabelText("单元格内容")).toHaveTextContent('"express": "SF"');

    // 重新执行:结果表格换了一个,旧的那一格不再显示,等选中新的格子。
    await user.click(header().getByRole("button", { name: "全部执行" }));
    await waitFor(() => expect(panel.getByText(/在表格里选中一格/)).toBeInTheDocument());
  });

  it("shows the cell of the result tab in front, not of the one switched away from", async () => {
    const { grid, user } = await runInShop("SELECT id, shipping FROM orders;\nSELECT name FROM customers;");
    await user.pointer({ keys: "[MouseRight]", target: cell(grid, 1, 1) });
    await user.click(screen.getByRole("menuitem", { name: "在右侧查看完整内容" }));
    const panel = within(screen.getByRole("complementary", { name: "信息" }));
    expect(panel.getByRole("heading", { name: "shipping" })).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: /结果 2/ }));
    const second = await screen.findByRole("grid", { name: "查询 1 的结果 2" });
    expect(panel.getByText(/在表格里选中一格/)).toBeInTheDocument();
    await user.click(cell(second, 0, 0));
    expect(panel.getByRole("heading", { name: "name" })).toBeInTheDocument();
    expect(panel.getByText(/查询 1 的结果 2 · 第 1 行$/)).toBeInTheDocument();
  });

  it("exports in the order the result is sorted by", async () => {
    const { user, grid } = await runInShop("SELECT id, name FROM customers LIMIT 5");
    await user.click(within(grid).getByRole("button", { name: /^name/ }));
    await user.click(within(grid).getByRole("button", { name: /^name/ }));
    await waitFor(() =>
      expect(within(grid).getByRole("columnheader", { name: /^name/ })).toHaveAttribute("aria-sort", "descending"),
    );
    await user.click(screen.getByRole("button", { name: "导出" }));
    const dialog = await screen.findByRole("dialog", { name: /导出/ });
    expect(dialog).toHaveTextContent("导出执行时读到的全部 5 行，按「name」降序。");
  });

  it("edits one table of a join at a time, cells only", async () => {
    const { user, grid } = await runInShop(
      "SELECT o.id, o.status, c.id, c.name FROM orders o JOIN customers c ON c.id = o.customer_id LIMIT 5",
    );
    const headers = within(grid)
      .getAllByRole("columnheader")
      .map((h) => h.textContent);
    // 表头是列名加类型;同名的 id 按表区分。
    const names = ["id（orders）", "status", "id（customers）", "name"];
    expect(headers.slice(1).map((h, i) => h?.startsWith(names[i]!))).toEqual([true, true, true, true]);
    const picker = await screen.findByRole("combobox", { name: "改哪张表" });
    expect(picker).toHaveDisplayValue("orders");
    expect(screen.queryByRole("button", { name: /新增行/ })).not.toBeInTheDocument();

    // 改 customers:orders 的列这时只读。
    await user.selectOptions(picker, "customers");
    const customerId = cell(grid, 0, 2).textContent;
    await user.dblClick(cell(grid, 0, 1));
    expect(screen.queryByRole("textbox", { name: "编辑「status」" })).not.toBeInTheDocument();
    await user.dblClick(cell(grid, 0, 3));
    const editor = screen.getByRole("textbox", { name: "编辑「name」" });
    await user.clear(editor);
    await user.type(editor, "连接里改的{Enter}");
    expect(picker).toBeDisabled();

    await user.pointer({ keys: "[MouseRight]", target: cell(grid, 0, 3) });
    const menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: /删除这一行/ })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");

    await user.keyboard("{Meta>}s{/Meta}");
    const statements = await screen.findByRole("list", { name: "将要执行的语句" });
    expect(statements).toHaveTextContent(
      `UPDATE \`shop\`.\`customers\` SET \`name\` = '连接里改的' WHERE \`id\` = ${customerId}`,
    );
  });

  it("edits a single-table result, commits it to the table and runs the statement again", async () => {
    const { user, grid, backend } = await runInShop("SELECT id, name AS who FROM customers LIMIT 5");
    const apply = vi.spyOn(backend, "applyTableChanges");
    const run = vi.spyOn(backend, "runStatement");
    expect(await screen.findByRole("button", { name: /新增行/ })).toBeEnabled();

    await user.dblClick(cell(grid, 0, 1));
    const editor = screen.getByRole("textbox", { name: "编辑「who」" });
    await user.clear(editor);
    await user.type(editor, "改过的名字{Enter}");
    expect(screen.getByText(/1 行有未提交的修改/)).toBeInTheDocument();
    expect(useTabs.getState().pending[useTabs.getState().activeId!]).toBe(1);

    await user.keyboard("{Meta>}s{/Meta}");
    const statements = await screen.findByRole("list", { name: "将要执行的语句" });
    expect(statements).toHaveTextContent("UPDATE `shop`.`customers` SET `name` = '改过的名字' WHERE `id` = 1");
    await user.click(screen.getByRole("button", { name: "提交" }));

    expect(await screen.findByText("已提交 1 条语句，影响 1 行。")).toBeInTheDocument();
    expect(apply.mock.calls[0]![2]).toEqual([
      { kind: "update", key: { id: { kind: "int", value: "1" } }, values: { name: "改过的名字" } },
    ]);
    expect(run).toHaveBeenCalledWith(
      expect.any(String),
      "mock-mysql",
      { database: "shop", schema: null },
      "SELECT id, name AS who FROM customers LIMIT 5",
    );
    expect(useTabs.getState().pending[useTabs.getState().activeId!]).toBeUndefined();
  });

  it("holds off running statements and switching results while edits are pending", async () => {
    const { user, grid } = await runInShop("SELECT * FROM customers LIMIT 3;\nSELECT 1 AS one;");
    await user.dblClick(cell(grid, 0, 1));
    const editor = screen.getByRole("textbox", { name: "编辑「name」" });
    await user.type(editor, "x{Enter}");

    expect(screen.getByRole("tab", { name: /结果 2/ })).toBeDisabled();
    expect(screen.getByRole("tab", { name: "消息" })).toBeDisabled();
    await user.click(header().getByRole("button", { name: /^执行/ }));
    expect(await screen.findByText("结果里还有没提交的修改：先提交或撤销，再执行。")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /撤销修改/ }));
    expect(screen.getByRole("tab", { name: /结果 2/ })).toBeEnabled();
  });

  it("explains why a result without the primary key is read-only", async () => {
    await runInShop("SELECT name FROM customers LIMIT 3");
    expect(await screen.findByText("结果里没有主键列 id，不能在这里改。")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /新增行/ })).not.toBeInTheDocument();
  });

  it("commits with ⌘S pressed straight from the cell editor, without saving the SQL", async () => {
    const { user, grid } = await runInShop("SELECT * FROM customers LIMIT 3");
    await user.dblClick(cell(grid, 0, 1));
    const editor = screen.getByRole("textbox", { name: "编辑「name」" });
    await user.clear(editor);
    await user.type(editor, "直接提交");
    await user.keyboard("{Meta>}s{/Meta}");
    const statements = await screen.findByRole("list", { name: "将要执行的语句" });
    expect(statements).toHaveTextContent("SET `name` = '直接提交'");
    expect(screen.queryByRole("dialog", { name: /保存查询/ })).not.toBeInTheDocument();
  });

  it("stops fetching more rows while a new row is being added", async () => {
    const { user, backend, grid } = await runInShop("SELECT * FROM orders");
    const more = vi.spyOn(backend, "resultRows");
    await user.click(screen.getByRole("button", { name: /新增行/ }));
    await waitFor(() => expect(screen.getByText(/1 行有未提交的修改/)).toBeInTheDocument());
    scrollTo(grid, 990);
    await waitFor(() => expect(cell(grid, 999, 0)).toBeInTheDocument());
    expect(screen.getByText("已显示 1,000 / 共 1,500 行")).toBeInTheDocument();
    expect(more).not.toHaveBeenCalled();
  });
});
