// 复制表、删除表(mock 后端):默认名字、只复制结构、重名报错、删表前关掉相关的标签页。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import { tableDataTab, useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { MYSQL_CUSTOMERS, openTableMenu, tree } from "../../test/tableMenu";
import { tableKey } from "../sidebar/treeKeys";
import { copiedNotice, copyName, qualifiedName, tabsOfTable } from "./tableOpsModel";

/** 展开「开发 → 本地 MySQL → shop → 表」,在 `table` 上右键选 `item`。 */
const tableMenu = (
  item: string,
  table = "customers",
  backend: Backend = createMockBackend(),
  before: () => void = () => undefined,
) => openTableMenu(item, { ...MYSQL_CUSTOMERS, table }, backend, before);

describe("table operations", { timeout: 15_000 }, () => {
  it("copies only the structure under a fresh name and selects the new table", async () => {
    const { user } = await tableMenu("复制表…");
    const dialog = within(screen.getByRole("dialog", { name: "复制表" }));
    expect(dialog.getByLabelText("新表名")).toHaveValue("customers_copy1");
    expect(dialog.getByText(/外键和触发器不会复制/)).toBeInTheDocument();
    await user.click(dialog.getByRole("radio", { name: /^仅结构/ }));
    await user.click(dialog.getByRole("button", { name: "复制" }));
    expect(await tree().findByRole("treeitem", { name: "customers_copy1" })).toBeInTheDocument();
    expect(useUi.getState().selectedKey).toBe(tableKey("mock-mysql", "shop", null, "customers_copy1"));
    expect(within(screen.getByRole("contentinfo")).getByRole("alert")).toHaveTextContent(
      "已复制为「customers_copy1」，只复制了结构。",
    );
  });

  it("keeps the dialog open when the name is taken", async () => {
    const { user } = await tableMenu("复制表…");
    const dialog = within(screen.getByRole("dialog", { name: "复制表" }));
    const name = dialog.getByLabelText("新表名");
    await user.clear(name);
    expect(dialog.getByRole("button", { name: "复制" })).toBeDisabled();
    await user.type(name, "orders{Enter}");
    expect(await dialog.findByRole("alert")).toHaveTextContent("already exists");
    await user.click(dialog.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("drops a table and closes the tabs that showed it", async () => {
    const backend = createMockBackend();
    const drop = vi.spyOn(backend, "dropTable");
    const { user } = await tableMenu("删除表…", "customers", backend, () =>
      useTabs.getState().open(tableDataTab("mock-mysql", "shop", null, "customers")),
    );
    const dialog = within(screen.getByRole("dialog", { name: "删除表「customers」？" }));
    expect(dialog.getByText(/不能撤销。开着的数据页和设计器会一起关掉/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "删除表" }));
    await waitFor(() => expect(tree().queryByRole("treeitem", { name: "customers" })).not.toBeInTheDocument());
    expect(drop).toHaveBeenCalledWith("mock-mysql", { database: "shop", schema: null, table: "customers" });
    expect(useTabs.getState().tabs).toEqual([]);
    expect(within(screen.getByRole("contentinfo")).getByRole("alert")).toHaveTextContent("已删除表「customers」。");
  });

  it("shows why the database refused to drop the table", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "dropTable").mockRejectedValue({
      code: "database",
      message: "Cannot drop table 'customers' referenced by a foreign key constraint",
      fields: [],
    });
    const { user } = await tableMenu("删除表…", "customers", backend);
    const dialog = within(screen.getByRole("dialog", { name: "删除表「customers」？" }));
    await user.click(dialog.getByRole("button", { name: "删除表" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("foreign key constraint");
    expect(tree().getByRole("treeitem", { name: "customers" })).toBeInTheDocument();
  });
});

describe("tableOpsModel", () => {
  it("numbers copies past the names already taken", () => {
    expect(copyName("users", ["users"])).toBe("users_copy1");
    expect(copyName("users", ["users_copy1", "users_copy2"])).toBe("users_copy3");
  });

  it("finds the data and design tabs of a table", () => {
    const table = { database: "shop", schema: null, table: "users" };
    const data = tableDataTab("c1", "shop", null, "users");
    expect(tabsOfTable(data, "c1", table)).toBe(true);
    expect(tabsOfTable(data, "c2", table)).toBe(false);
    expect(tabsOfTable(tableDataTab("c1", "shop", null, "orders"), "c1", table)).toBe(false);
    const design = { kind: "tableDesign", id: "d", title: "users", connectionId: "c1", ...table } as const;
    expect(tabsOfTable(design, "c1", table)).toBe(true);
    expect(tabsOfTable({ ...design, table: null }, "c1", table)).toBe(false);
  });

  it("describes the outcome", () => {
    const table = { database: "FOLIO", schema: "public", table: "users_copy1" };
    expect(copiedNotice({ table, rows: 12345 })).toBe("已复制为「users_copy1」，复制了 12,345 行。");
    expect(qualifiedName(table)).toBe("FOLIO.public.users_copy1");
  });
});
