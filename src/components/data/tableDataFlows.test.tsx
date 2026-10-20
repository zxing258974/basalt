// 表数据页的交互流程:编辑 → ⌘S 预览 → 提交、失败保留修改、只读表、筛选与锁定翻页。
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ConnectionConfig } from "../../lib/types";
import { closeTabGuarded } from "../../stores/discard";
import { tableDataTab, useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { DiscardChangesDialog } from "./DiscardChangesDialog";
import { TableDataView } from "./TableDataView";

const local: ConnectionConfig = {
  id: "local",
  name: "本地",
  driver: "mysql",
  endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root" },
  savePassword: true,
};

// jsdom 不做布局,虚拟滚动量到的容器尺寸是 0,一行都不渲染;这里给个固定尺寸。
const sizes = { clientHeight: 600, clientWidth: 1000 } as const;
const originals = Object.keys(sizes).map((key) => Object.getOwnPropertyDescriptor(Element.prototype, key));
beforeAll(() => {
  for (const [key, value] of Object.entries(sizes)) {
    Object.defineProperty(Element.prototype, key, { configurable: true, get: () => value });
  }
});
afterAll(() => {
  Object.keys(sizes).forEach((key, i) => Object.defineProperty(Element.prototype, key, originals[i]!));
});
afterEach(() => setBackendForTesting(null));

async function renderTable(table = "customers") {
  const backend = createMockBackend([local]);
  await backend.openConnection("local");
  setBackendForTesting(backend);
  const client = createQueryClient();
  const tab = tableDataTab("local", "shop", null, table);
  useTabs.setState({ tabs: [tab], activeId: tab.id, pending: {} });
  useUi.setState({ discardPrompt: null });
  render(
    <QueryClientProvider client={client}>
      <TableDataView tab={tab} active />
      <DiscardChangesDialog />
    </QueryClientProvider>,
  );
  const grid = await screen.findByRole("grid", {}, { timeout: 3000 });
  await waitFor(() => expect(within(grid).getAllByRole("row").length).toBeGreaterThan(1));
  return { backend, grid, user: userEvent.setup() };
}

/** 第 row 个数据行(从 0 数)的第 col 个格子。 */
function cell(grid: HTMLElement, row: number, col: number): HTMLElement {
  const dataRows = within(grid)
    .getAllByRole("row")
    .filter((r) => r.getAttribute("aria-rowindex") === String(row + 2));
  return within(dataRows[0]!).getAllByRole("gridcell")[col]!;
}

const readName = async (backend: ReturnType<typeof createMockBackend>) => {
  const page = await backend.readTablePage(
    "local",
    { database: "shop", schema: null, table: "customers" },
    {
      offset: 0,
      limit: 1,
      sort: null,
      filter: null,
    },
  );
  return page.rows[0]![1];
};

describe("table data", () => {
  it("edits a cell, previews the UPDATE with ⌘S and commits it", async () => {
    const { backend, grid, user } = await renderTable();
    await user.dblClick(cell(grid, 0, 1));
    const editor = screen.getByRole("textbox", { name: "编辑「name」" });
    await user.clear(editor);
    await user.type(editor, "测试名{Enter}");

    expect(cell(grid, 0, 1)).toHaveClass("is-edited");
    expect(cell(grid, 0, 1)).toHaveTextContent("测试名");
    expect(screen.getByText(/1 行有未提交的修改/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "下一页" })).toBeDisabled();

    await user.keyboard("{Meta>}s{/Meta}");
    const statements = await screen.findByRole("list", { name: "将要执行的语句" });
    expect(statements).toHaveTextContent("UPDATE `shop`.`customers` SET `name` = '测试名' WHERE `id` = 1");

    await user.click(screen.getByRole("button", { name: "提交" }));
    expect(await screen.findByText("已提交 1 条语句，影响 1 行。")).toBeInTheDocument();
    await waitFor(() => expect(cell(grid, 0, 1)).not.toHaveClass("is-edited"));
    expect(cell(grid, 0, 1)).toHaveTextContent("测试名");
    expect(await readName(backend)).toEqual({ kind: "text", value: "测试名" });
  });

  it("keeps the edits and shows why when the commit is rejected", async () => {
    // 行数少的表:jsdom 不会真的滚动,新行要落在已渲染的范围里。
    const { grid, user } = await renderTable("widgets");
    await user.click(screen.getByRole("button", { name: "新增行" }));
    await waitFor(() => expect(within(grid).getAllByRole("row").at(-1)).toHaveClass("is-new"));
    const newRow = within(grid).getAllByRole("row").at(-1)!;
    await user.dblClick(within(newRow).getAllByRole("gridcell")[0]!);
    await user.type(screen.getByRole("textbox", { name: "编辑「id」" }), "1{Enter}");

    await user.click(screen.getByRole("button", { name: /提交/ }));
    await screen.findByRole("list", { name: "将要执行的语句" });
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "提交" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Duplicate entry");
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText(/1 行有未提交的修改/)).toBeInTheDocument();
  });

  it("marks a row for deletion, restores it and reverts everything", async () => {
    const { grid, user } = await renderTable();
    await user.click(cell(grid, 2, 1));
    await user.click(screen.getByRole("button", { name: "删除行" }));
    expect(cell(grid, 2, 1).closest("[role=row]")).toHaveClass("is-deleted");

    await user.click(screen.getByRole("button", { name: "恢复行" }));
    expect(cell(grid, 2, 1).closest("[role=row]")).not.toHaveClass("is-deleted");

    await user.click(cell(grid, 2, 1));
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    expect(cell(grid, 2, 1).closest("[role=row]")).toHaveClass("is-deleted");
    await user.dblClick(cell(grid, 0, 3));
    await user.type(screen.getByRole("textbox", { name: "编辑「city」" }), "{Escape}");
    expect(screen.getByText(/1 行有未提交的修改/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "撤销修改" }));
    expect(
      within(grid)
        .queryAllByRole("row")
        .some((r) => r.classList.contains("is-deleted")),
    ).toBe(false);
    expect(screen.queryByText(/未提交的修改/)).not.toBeInTheDocument();
  });

  it("keeps NULL when the editor is closed without typing", async () => {
    const { grid, user } = await renderTable();
    const target = within(grid)
      .getAllByRole("gridcell")
      .find((c) => c.classList.contains("is-null"))!;
    await user.dblClick(target);
    await user.keyboard("{Enter}");
    expect(target).not.toHaveClass("is-edited");
    expect(screen.queryByText(/未提交的修改/)).not.toBeInTheDocument();
  });

  it("explains why a table without a primary key is read-only", async () => {
    const { user, grid } = await renderTable("v_daily_sales");
    expect(screen.getByText(/没有主键/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "新增行" })).toBeDisabled();
    await user.dblClick(cell(grid, 0, 1));
    expect(screen.queryByRole("textbox", { name: /编辑/ })).not.toBeInTheDocument();
  });

  it("filters on the server and offers to clear a filter that fails", async () => {
    const { grid, user } = await renderTable();
    const filter = screen.getByRole("textbox", { name: "筛选条件（WHERE 子句）" });
    await user.type(filter, "city = '上海'{Enter}");
    await waitFor(() => expect(screen.getByText(/WHERE \(/)).toBeInTheDocument());
    const cities = within(grid)
      .getAllByRole("row")
      .slice(1)
      .map((row) => within(row).getAllByRole("gridcell")[3]!.textContent);
    expect(new Set(cities)).toEqual(new Set(["上海"]));

    await user.clear(filter);
    await user.type(filter, "nonsense{Enter}");
    expect(await screen.findByText("读取数据失败")).toBeInTheDocument();
    // 筛选框里的 × 和占位里的按钮都能清除,点哪个都行。
    await user.click(screen.getAllByRole("button", { name: "清除筛选" })[0]!);
    expect(await screen.findByRole("grid")).toBeInTheDocument();
    expect(filter).toHaveValue("");
  });

  it("sorts by clicking a column header", async () => {
    const { grid, user } = await renderTable();
    await user.click(screen.getByRole("button", { name: /^balance/ }));
    await waitFor(() => expect(screen.getByText(/ORDER BY `balance`, `id`/)).toBeInTheDocument());
    const header = within(grid)
      .getAllByRole("columnheader")
      .find((h) => h.textContent?.includes("balance"))!;
    expect(header).toHaveAttribute("aria-sort", "ascending");
  });

  it("asks before closing a tab that has unsaved edits", async () => {
    const { grid, user } = await renderTable();
    const tabId = useTabs.getState().tabs[0]!.id;
    await user.dblClick(cell(grid, 0, 3));
    await user.type(screen.getByRole("textbox", { name: "编辑「city」" }), "x{Enter}");

    closeTabGuarded(tabId);
    expect(await screen.findByText("有 1 行修改还没有提交，关闭后就没有了。")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(useTabs.getState().tabs).toHaveLength(1);

    closeTabGuarded(tabId);
    await user.click(await screen.findByRole("button", { name: "放弃修改并关闭" }));
    expect(useTabs.getState().tabs).toHaveLength(0);
  });

  it("does not reopen the preview when it was cancelled while loading", async () => {
    const { grid, user } = await renderTable();
    await user.dblClick(cell(grid, 0, 3));
    await user.type(screen.getByRole("textbox", { name: "编辑「city」" }), "x{Enter}");
    await user.keyboard("{Meta>}s{/Meta}");
    await user.click(await screen.findByRole("button", { name: "取消" }));
    // 等过 mock 后端生成预览的延迟,迟到的结果不能再把对话框弹出来。
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
