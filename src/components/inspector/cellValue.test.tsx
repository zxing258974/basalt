// 信息面板「值」页(mock 后端):选中格子就显示完整内容、JSON 排版与原文、未提交的新值、
// 右键打开、只显示当前标签页的、表格没了就清掉。
import { QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ConnectionConfig } from "../../lib/types";
import { useCellPeek } from "../../stores/cellPeek";
import { useInspector } from "../../stores/inspector";
import { tableDataTab, useTabs } from "../../stores/tabs";
import { TableDataView } from "../data/TableDataView";
import { TabScope } from "../layout/tabScope";
import { InspectorPanel } from "./InspectorPanel";

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

const panel = () => within(screen.getByRole("complementary", { name: "信息" }));

async function renderOrders() {
  const backend = createMockBackend([local]);
  await backend.openConnection("local");
  setBackendForTesting(backend);
  const tab = tableDataTab("local", "shop", null, "orders");
  useTabs.setState({ tabs: [tab], activeId: tab.id, pending: {} });
  const view = render(
    <QueryClientProvider client={createQueryClient()}>
      <TabScope.Provider value={{ tabId: tab.id, active: true }}>
        <TableDataView tab={tab} active />
      </TabScope.Provider>
      <div className="app-shell has-inspector">
        <InspectorPanel />
      </div>
    </QueryClientProvider>,
  );
  const grid = await screen.findByRole("grid", {}, { timeout: 3000 });
  await waitFor(() => expect(within(grid).getAllByRole("row").length).toBeGreaterThan(1));
  return { grid, tab, view, user: userEvent.setup() };
}

/** 第 row 个数据行(从 0 数)的第 col 个格子。 */
function cell(grid: HTMLElement, row: number, col: number): HTMLElement {
  const dataRow = within(grid)
    .getAllByRole("row")
    .find((r) => r.getAttribute("aria-rowindex") === String(row + 2))!;
  return within(dataRow).getAllByRole("gridcell")[col]!;
}

/** orders 的第 2 行(mock 数据固定)运费是 JSON。 */
const SHIPPING = 5;

describe("cell value pane", () => {
  beforeEach(() => {
    localStorage.clear();
    act(() => useInspector.setState({ open: true, tab: "value" }));
  });
  afterEach(() => {
    setBackendForTesting(null);
    vi.restoreAllMocks();
  });

  it("shows the whole selected cell and lays out JSON", async () => {
    const { grid, user } = await renderOrders();
    expect(panel().getByText(/在表格里选中一格/)).toBeInTheDocument();

    await user.click(cell(grid, 1, SHIPPING));
    expect(panel().getByRole("heading", { name: "shipping" })).toBeInTheDocument();
    expect(panel().getByText("json · orders · 第 2 行")).toBeInTheDocument();
    const content = panel().getByLabelText("单元格内容");
    expect(content.textContent).toMatch(/^\{\n {2}"city": "[^"]+",\n {2}"express": "SF"\n\}$/);
    expect(panel().getByText(/^\d+ 字符$/)).toBeInTheDocument();

    await user.click(panel().getByRole("button", { name: "原文" }));
    expect(panel().getByLabelText("单元格内容").textContent).toMatch(/^\{"city":"[^"]+","express":"SF"\}$/);
    expect(panel().getByRole("button", { name: "原文" })).toHaveAttribute("aria-pressed", "true");

    const write = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    await user.click(panel().getByRole("button", { name: "复制" }));
    expect(write).toHaveBeenCalledWith(panel().getByLabelText("单元格内容").textContent);
    expect(await panel().findByRole("button", { name: "已复制" })).toBeInTheDocument();

    await user.click(cell(grid, 0, 0));
    expect(panel().getByRole("heading", { name: "id" })).toBeInTheDocument();
    expect(panel().queryByRole("group", { name: "JSON 显示方式" })).not.toBeInTheDocument();
  });

  it("shows NULL and an uncommitted new value for what they are", async () => {
    const { grid, user } = await renderOrders();
    await user.click(cell(grid, 0, SHIPPING));
    expect(panel().getByText(/NULL：这一格没有值/)).toBeInTheDocument();

    await user.pointer({ keys: "[MouseRight]", target: cell(grid, 0, 3) });
    await user.click(screen.getByRole("menuitem", { name: "设为空字符串" }));
    expect(panel().getByText("空字符串")).toBeInTheDocument();
    expect(panel().getByText("未提交")).toBeInTheDocument();
  });

  it("marks a cell whose row is about to be deleted", async () => {
    const { grid, user } = await renderOrders();
    await user.pointer({ keys: "[MouseRight]", target: cell(grid, 1, SHIPPING) });
    await user.click(screen.getByRole("menuitem", { name: /^删除这一行/ }));
    expect(panel().getByText("将删除")).toBeInTheDocument();
  });

  it("shows the start of a huge value and the rest on request, copying all of it", async () => {
    const user = userEvent.setup();
    const huge = `${"长".repeat(200_005)}尾`;
    useTabs.setState({ tabs: [], activeId: "t1", pending: {} });
    useCellPeek.getState().publish("t1", "test", {
      source: "logs",
      column: "body",
      dataType: "longtext",
      rowNumber: 1,
      value: { kind: "text", text: huge },
      edited: false,
      deleted: false,
    });
    render(
      <div className="app-shell has-inspector">
        <InspectorPanel />
      </div>,
    );
    expect(panel().getByLabelText("单元格内容").textContent).toHaveLength(200_000);
    expect(panel().getByText(/只显示了前 200,000 个字符/)).toBeInTheDocument();
    expect(panel().getByText("200,006 字符")).toBeInTheDocument();

    const write = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    await user.click(panel().getByRole("button", { name: "复制" }));
    expect(write).toHaveBeenCalledWith(huge);
    expect(await panel().findByRole("status")).toHaveTextContent("已复制到剪贴板");

    await user.click(panel().getByRole("button", { name: "显示全部" }));
    expect(panel().getByLabelText("单元格内容").textContent).toBe(huge);
    expect(panel().queryByText(/只显示了前/)).not.toBeInTheDocument();
  });

  it("opens on the value tab from the cell menu", async () => {
    act(() => useInspector.setState({ open: false, tab: "properties" }));
    const { grid, user } = await renderOrders();
    await user.pointer({ keys: "[MouseRight]", target: cell(grid, 1, SHIPPING) });
    await user.click(screen.getByRole("menuitem", { name: "在右侧查看完整内容" }));
    expect(useInspector.getState()).toMatchObject({ open: true, tab: "value" });
    expect(localStorage.getItem("basalt.inspector.tab")).toBe("value");
    expect(panel().getByRole("heading", { name: "shipping" })).toBeInTheDocument();
  });

  it("only shows the cell of the tab in front, and forgets it when the grid goes", async () => {
    const { grid, tab, view, user } = await renderOrders();
    await user.click(cell(grid, 1, SHIPPING));
    act(() => useTabs.setState({ activeId: "another-tab" }));
    expect(panel().getByText(/在表格里选中一格/)).toBeInTheDocument();
    act(() => useTabs.setState({ activeId: tab.id }));
    expect(panel().getByRole("heading", { name: "shipping" })).toBeInTheDocument();

    view.unmount();
    expect(useCellPeek.getState().peek).toBeNull();
  });
});
