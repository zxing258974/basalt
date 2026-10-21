// 右侧信息面板(mock 后端):跟着对象树的选中走、属性与建表语句两页、开关记在本机、读不到时说原因。
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import { useInspector } from "../../stores/inspector";
import { useUi } from "../../stores/ui";
import { renderApp } from "../../test/renderApp";
import { tableFromKey, tableKey } from "../sidebar/treeKeys";
import { formatProperty } from "./inspectorModel";

const panel = () => within(screen.getByRole("complementary", { name: "信息" }));
const toggle = () => within(screen.getByRole("banner")).getByRole("button", { name: "信息" });

/** renderApp 会把 store 复位成初始状态(窄窗口里面板默认收着):渲染之后再把面板打开。 */
const showInspector = () => act(() => useInspector.setState({ open: true, tab: "properties" }));

async function openShop() {
  const app = renderApp();
  showInspector();
  await app.backend.openConnection("mock-mysql");
  return app;
}

describe("inspector panel", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.restoreAllMocks());

  it("follows the selected table and switches between properties and DDL", async () => {
    const { user } = await openShop();
    expect(panel().getByText(/选中一张表或视图/)).toBeInTheDocument();
    act(() => useUi.getState().select(tableKey("mock-mysql", "shop", null, "customers")));
    expect(await panel().findByText("InnoDB")).toBeInTheDocument();
    expect(panel().getByRole("heading", { name: "customers" })).toBeInTheDocument();
    expect(panel().getByText("2.5 MB（2,637,824）")).toBeInTheDocument();

    await user.click(panel().getByRole("tab", { name: "DDL" }));
    expect(panel().getByLabelText("建表语句")).toHaveTextContent(/^CREATE TABLE `shop`.`customers`/);
    expect(localStorage.getItem("basalt.inspector.tab")).toBe("ddl");
    const write = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    await user.click(panel().getByRole("button", { name: "复制" }));
    expect(write).toHaveBeenCalledWith(expect.stringContaining("CREATE TABLE"));
    expect(await panel().findByRole("button", { name: "已复制" })).toBeInTheDocument();

    act(() => useUi.getState().select(tableKey("mock-mysql", "shop", null, "orders")));
    expect(await panel().findByRole("heading", { name: "orders" })).toBeInTheDocument();
    expect(await panel().findByRole("button", { name: "复制" })).toBeInTheDocument();
  });

  it("toggles from the toolbar and remembers it", async () => {
    const { user } = await openShop();
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    await user.click(toggle());
    expect(screen.queryByRole("complementary", { name: "信息" })).not.toBeInTheDocument();
    expect(localStorage.getItem("basalt.inspector.open")).toBe("0");
    await user.click(toggle());
    expect(screen.getByRole("complementary", { name: "信息" })).toBeInTheDocument();
    expect(localStorage.getItem("basalt.inspector.open")).toBe("1");
  });

  it("can be dragged wider from its left edge, like the sidebar", async () => {
    HTMLElement.prototype.setPointerCapture = vi.fn();
    const { user, container } = await openShop();
    const shell = container.querySelector<HTMLElement>(".app-shell")!;
    const resizer = screen.getByRole("separator", { name: "调整信息面板宽度" });
    expect(shell.style.getPropertyValue("--inspector-width")).toBe("280px");
    fireEvent.pointerDown(resizer, { clientX: 900, pointerId: 1 });
    fireEvent.pointerMove(resizer, { clientX: 780 });
    fireEvent.pointerUp(resizer);
    expect(shell.style.getPropertyValue("--inspector-width")).toBe("400px");
    expect(localStorage.getItem("basalt.inspectorWidth")).toBe("400");

    await user.click(toggle());
    expect(screen.queryByRole("separator", { name: "调整信息面板宽度" })).not.toBeInTheDocument();
    expect(shell.style.getPropertyValue("--inspector-width")).toBe("0px");
    await user.click(toggle());
    expect(shell.style.getPropertyValue("--inspector-width")).toBe("400px");
  });

  it("explains why the details could not be read", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "tableDetails").mockRejectedValue({ code: "database", message: "没有权限", fields: [] });
    renderApp(backend);
    showInspector();
    act(() => useUi.getState().select(tableKey("mock-mysql", "shop", null, "customers")));
    await waitFor(() => expect(panel().getByRole("alert")).toHaveTextContent("没有权限"));
  });
});

describe("inspector model", () => {
  it("formats every kind of value", () => {
    expect(formatProperty(null)).toBe("—");
    expect(formatProperty({ kind: "text", value: "InnoDB" })).toBe("InnoDB");
    expect(formatProperty({ kind: "dateTime", value: "2026-08-10 09:47:02" })).toBe("2026-08-10 09:47:02");
    expect(formatProperty({ kind: "count", value: 18204 })).toBe("18,204");
    expect(formatProperty({ kind: "bytes", value: 512 })).toBe("512 B");
    expect(formatProperty({ kind: "bytes", value: 16384 })).toBe("16 KB（16,384）");
  });

  it("reads tables out of tree keys", () => {
    expect(tableFromKey(tableKey("c1", "db", "public", "a/b"))).toEqual({
      connectionId: "c1",
      database: "db",
      schema: "public",
      table: "a/b",
    });
    expect(tableFromKey(tableKey("c1", "shop", null, "users"))?.schema).toBeNull();
    expect(tableFromKey("c:c1/d:shop")).toBeNull();
    expect(tableFromKey(null)).toBeNull();
  });
});
