// 表数据页的交互流程:编辑 → ⌘S 预览 → 提交、失败保留修改、只读表、筛选与锁定翻页、选中格直接打字(含输入法)、行数变少后选区收紧。
import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
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
    await user.click(cell(grid, 0, 1));
    await user.keyboard("x");
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

describe("typing on a selected cell", () => {
  const editorFor = (column: string) => screen.queryByRole("textbox", { name: `编辑「${column}」` });

  /** 点选格子,返回接收键盘的输入框:它应该就是当前焦点。 */
  async function selectCell(grid: HTMLElement, user: UserEvent, row: number, col: number, table = "customers") {
    await user.click(cell(grid, row, col));
    const input = screen.getByRole("textbox", { name: `${table} 的数据` });
    expect(input).toHaveFocus();
    return input;
  }

  /** 输入法组字:keyCode 229 的按键归输入法,组字中的文字先进输入框。 */
  function compose(input: HTMLElement, ...steps: string[]) {
    fireEvent.keyDown(input, { key: steps[0]?.at(-1) ?? "n", keyCode: 229 });
    fireEvent.compositionStart(input, { data: "" });
    for (const value of steps) {
      fireEvent.input(input, { target: { value }, inputType: "insertCompositionText", isComposing: true });
    }
  }

  it("starts editing with a typed character and keeps the rest of the typing", async () => {
    const { grid, user } = await renderTable();
    await selectCell(grid, user, 0, 3);
    await user.keyboard("xy");
    expect(editorFor("city")).toHaveValue("xy");
    await user.keyboard("{Enter}");
    expect(cell(grid, 0, 3)).toHaveTextContent("xy");
    expect(cell(grid, 0, 3)).toHaveClass("is-edited");
  });

  // 结束组字的事件顺序:Chromium 先发最后一个组字中的 input 再 compositionend;
  // WebKit 先发 compositionend(文字已经在框里),之后再补一个不算组字的 input。
  it.each([
    [
      "input, then compositionend",
      (input: HTMLElement) => {
        fireEvent.input(input, { target: { value: "你" }, inputType: "insertCompositionText", isComposing: true });
        fireEvent.compositionEnd(input, { data: "你" });
      },
    ],
    [
      "compositionend, then input",
      (input: HTMLElement) => {
        (input as HTMLTextAreaElement).value = "你";
        fireEvent.compositionEnd(input, { data: "你" });
        fireEvent.input(input, { inputType: "insertText" });
      },
    ],
  ])("composes with an input method before opening the editor (%s)", async (_order, finish) => {
    const { grid, user } = await renderTable();
    const input = await selectCell(grid, user, 0, 3);
    compose(input, "n", "ni");
    // 组字期间挑候选、回车上屏的键都归输入法:不移动选区,也不打开编辑框。
    fireEvent.keyDown(input, { key: "ArrowDown", keyCode: 229, isComposing: true });
    fireEvent.keyDown(input, { key: "Enter", keyCode: 229, isComposing: true });
    expect(editorFor("city")).not.toBeInTheDocument();
    expect(input).toHaveAttribute("aria-activedescendant", cell(grid, 0, 3).id);

    finish(input);
    const editor = editorFor("city");
    expect(editor).toHaveValue("你");
    expect(editor).toHaveFocus();
    await user.keyboard("好{Enter}");
    expect(cell(grid, 0, 3)).toHaveTextContent("你好");
  });

  it("drops a cancelled composition and keeps the keyboard on the grid", async () => {
    const { grid, user } = await renderTable();
    const input = await selectCell(grid, user, 0, 3);
    compose(input, "n", "ni", "");
    fireEvent.compositionEnd(input, { data: "" });
    expect(editorFor("city")).not.toBeInTheDocument();
    expect(input).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(input).toHaveAttribute("aria-activedescendant", cell(grid, 1, 3).id);
    // 取消掉的组字不能把之后的输入带回原来那一格。
    await user.keyboard("x");
    expect(cell(grid, 1, 3)).toContainElement(editorFor("city"));
  });

  it("starts editing with text an input method commits without composing", async () => {
    // 拼音模式下的中文标点直接上屏,不经过组字。
    const { grid, user } = await renderTable();
    const input = await selectCell(grid, user, 0, 3);
    fireEvent.keyDown(input, { key: ",", keyCode: 229 });
    fireEvent.input(input, { target: { value: "，" }, inputType: "insertText" });
    expect(editorFor("city")).toHaveValue("，");
  });

  it("keeps the grid shortcuts, the tab order and the active cell on the keyboard input", async () => {
    const { grid, user } = await renderTable();
    const input = await selectCell(grid, user, 0, 1);
    expect(input).toHaveAttribute("aria-activedescendant", cell(grid, 0, 1).id);

    await user.keyboard("{Meta>}c{/Meta}");
    expect(await navigator.clipboard.readText()).toBe(cell(grid, 0, 1).textContent);
    await user.paste("粘贴的文字");
    expect(editorFor("name")).not.toBeInTheDocument();

    await user.keyboard("{ArrowRight}");
    expect(input).toHaveAttribute("aria-activedescendant", cell(grid, 0, 2).id);
    // 滚动容器不占 Tab 位:往回 Tab 一次就离开表格,不会停在滚动容器上再被转回来。
    await user.tab({ shift: true });
    expect(document.activeElement).not.toBe(document.body);
    expect(input.closest(".data-grid")).not.toContainElement(document.activeElement as HTMLElement);
  });

  it("lets the input method run on a read-only cell but drops the text", async () => {
    const { grid, user } = await renderTable("v_daily_sales");
    const input = await selectCell(grid, user, 0, 1, "v_daily_sales");
    compose(input, "n", "ni");
    expect(input).not.toHaveClass("is-composing");
    fireEvent.compositionEnd(input, { data: "你" });
    expect(screen.queryByRole("textbox", { name: /编辑/ })).not.toBeInTheDocument();
    expect(input).toHaveFocus();
  });

  it("does not open the editor for text that arrives after the input lost focus", async () => {
    const { grid, user } = await renderTable();
    const input = await selectCell(grid, user, 0, 3);
    compose(input, "n", "ni");
    expect(input).toHaveClass("is-composing");
    await user.click(screen.getByRole("textbox", { name: "筛选条件（WHERE 子句）" }));
    expect(input).not.toHaveClass("is-composing");
    fireEvent.compositionEnd(input, { data: "你" });
    expect(editorFor("city")).not.toBeInTheDocument();
  });

  it("types straight into a new row and returns to the grid after the context menu", async () => {
    const { grid, user } = await renderTable("widgets");
    await user.click(screen.getByRole("button", { name: "新增行" }));
    await waitFor(() => expect(within(grid).getAllByRole("row").at(-1)).toHaveClass("is-new"));
    await user.keyboard("7");
    const newRow = within(grid).getAllByRole("row").at(-1)!;
    // 新行选中的是第一个要手填的格子(跳过自增和有默认值的列)。
    expect(within(newRow).getByRole("textbox", { name: /^编辑/ })).toHaveValue("7");
    await user.keyboard("{Escape}");

    await user.pointer({ keys: "[MouseRight]", target: cell(grid, 0, 1) });
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "widgets 的数据" })).toHaveFocus();
  });
});

// widgets 是 25 行的通用表(id、name、updated_at),新行选中的是第一个要手填的 name 列。
describe("selection after rows go away", () => {
  it("moves the selection to the last row when an added row is reverted", async () => {
    const { grid, user } = await renderTable("widgets");
    await user.click(screen.getByRole("button", { name: "新增行" }));
    await user.click(screen.getByRole("button", { name: "撤销修改" }));
    const input = screen.getByRole("textbox", { name: "widgets 的数据" });
    act(() => input.focus());
    expect(input).toHaveAttribute("aria-activedescendant", cell(grid, 24, 1).id);
    expect(screen.getByRole("button", { name: "删除行" })).toBeEnabled();
    await user.keyboard("{Meta>}c{/Meta}");
    expect(await navigator.clipboard.readText()).toBe("widgets #25");
    await user.keyboard("{F2}");
    expect(cell(grid, 24, 1)).toContainElement(screen.getByRole("textbox", { name: "编辑「name」" }));
  });

  it("keeps the selection on a remaining row after a deleted row is committed", async () => {
    const { grid, user } = await renderTable("widgets");
    await user.click(cell(grid, 24, 1));
    await user.keyboard("{Meta>}{Backspace}{/Meta}{Meta>}s{/Meta}");
    await screen.findByRole("list", { name: "将要执行的语句" });
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "提交" }));
    expect(await screen.findByText("已提交 1 条语句，影响 1 行。")).toBeInTheDocument();
    // 提交后焦点回到表格,原来选中的第 25 行已经没了。
    await user.keyboard("{Enter}");
    expect(cell(grid, 23, 1)).toContainElement(screen.getByRole("textbox", { name: "编辑「name」" }));
  });
});
