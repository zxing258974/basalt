// 对象列表页:排序、类型筛选、关键字筛选,以及连接关闭 / 读取失败 / 为空时的提示。
import { act, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import { objectsTab, useTabs } from "../../stores/tabs";
import { renderApp } from "../../test/renderApp";

async function showObjects(
  connectionId: string,
  database: string,
  schema: string | null = null,
  backend: Backend = createMockBackend(),
  { open = true } = {},
) {
  if (open) await backend.openConnection(connectionId, "pw");
  const context = renderApp(backend);
  act(() => useTabs.getState().open(objectsTab(connectionId, database, schema)));
  return context;
}

const rowNames = () =>
  within(screen.getByRole("table"))
    .getAllByRole("row")
    .slice(1)
    .map((row) => within(row).getAllByRole("cell")[0]?.textContent);

describe("listing", () => {
  it("lists tables and views by name with a summary", async () => {
    await showObjects("mock-mysql", "shop");
    expect(await screen.findByText("4 张表 · 1 个视图")).toBeInTheDocument();
    expect(rowNames()).toEqual(["customers", "order_items", "orders", "products", "v_daily_sales"]);
    expect(screen.getByRole("columnheader", { name: "引擎" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "名称" })).toHaveAttribute("aria-sort", "ascending");
  });

  it("sorts by a numeric column largest first, then flips", async () => {
    const { user } = await showObjects("mock-mysql", "shop");
    await screen.findByRole("table");
    await user.click(screen.getByRole("button", { name: "行数(估)" }));
    expect(screen.getByRole("columnheader", { name: "行数(估)" })).toHaveAttribute("aria-sort", "descending");
    expect(rowNames().slice(0, 2)).toEqual(["order_items", "orders"]);

    await user.click(screen.getByRole("button", { name: "行数(估)" }));
    expect(screen.getByRole("columnheader", { name: "行数(估)" })).toHaveAttribute("aria-sort", "ascending");
    expect(rowNames()[0]).toBe("products");

    await user.click(screen.getByRole("button", { name: "名称" }));
    expect(rowNames()[0]).toBe("customers");
  });

  it("filters by kind and by name or comment", async () => {
    const { user } = await showObjects("mock-mysql", "shop");
    await screen.findByRole("table");
    await user.click(screen.getByRole("radio", { name: "视图" }));
    expect(screen.getByRole("radio", { name: "视图" })).toBeChecked();
    expect(rowNames()).toEqual(["v_daily_sales"]);

    await user.click(screen.getByRole("radio", { name: "全部" }));
    await user.type(screen.getByLabelText("筛选对象"), "订单");
    expect(rowNames()).toEqual(["orders"]);

    await user.type(screen.getByLabelText("筛选对象"), "不存在");
    expect(screen.getByText("没有符合筛选条件的对象")).toBeInTheDocument();
  });

  it("refetches on refresh", async () => {
    const backend = createMockBackend();
    const listTables = vi.spyOn(backend, "listTables");
    const { user } = await showObjects("mock-mysql", "shop", null, backend);
    await screen.findByRole("table");
    const objects = within(screen.getByRole("region", { name: "shop 的对象" }));
    await user.click(objects.getByRole("button", { name: "刷新" }));
    expect(listTables).toHaveBeenCalledTimes(2);
  });

  it("shows a schema's objects with the columns PostgreSQL reports", async () => {
    await showObjects("mock-pg", "folio", "diku_mod_inventory_storage");
    expect(await screen.findByRole("heading", { name: "folio.diku_mod_inventory_storage" })).toBeInTheDocument();
    expect(await screen.findByText("物化视图")).toBeInTheDocument();
    expect(screen.queryByRole("columnheader", { name: "引擎" })).not.toBeInTheDocument();
  });
});

describe("states", () => {
  it("offers to reopen a closed connection", async () => {
    const { user } = await showObjects("mock-mysql", "shop", null, createMockBackend(), { open: false });
    expect(await screen.findByText("连接已关闭")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重新打开" }));
    expect(await screen.findByRole("table")).toBeInTheDocument();
  });

  it("explains a tab whose connection was deleted", async () => {
    await showObjects("gone", "shop", null, createMockBackend(), { open: false });
    expect(await screen.findByText("这个连接已经不存在")).toBeInTheDocument();
    expect(screen.getByText("已关闭的连接")).toBeInTheDocument();
  });

  it("shows why the objects could not be read", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "listTables").mockRejectedValue({ code: "query_failed", message: "权限不足", fields: [] });
    await showObjects("mock-mysql", "shop", null, backend);
    expect(await screen.findByText("读取对象失败")).toBeInTheDocument();
    expect(screen.getByText("权限不足")).toBeInTheDocument();
  });

  it("shows a loading state, then an empty schema", async () => {
    const backend = createMockBackend();
    let finish: (value: []) => void = () => {};
    vi.spyOn(backend, "listTables").mockReturnValue(new Promise((resolve) => (finish = resolve)));
    await showObjects("mock-pg", "folio", "public", backend);
    expect(await screen.findByText("正在读取对象…")).toBeInTheDocument();
    act(() => finish([]));
    expect(await screen.findByText("这里还没有任何表或视图")).toBeInTheDocument();
  });
});
