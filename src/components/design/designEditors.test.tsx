// 设计器各页的编辑:字段表格、类型下拉、索引、外键、选项、SQL 预览。
import { QueryClientProvider } from "@tanstack/react-query";
import { act, screen, waitFor, within } from "@testing-library/react";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ConnectionConfig, DriverKind } from "../../lib/types";
import { newTableTab, tableDesignTab, useTabs, type TableDesignTab } from "../../stores/tabs";
import { TableDesignView } from "./TableDesignView";

function connection(driver: DriverKind): ConnectionConfig {
  return driver === "sqlite"
    ? { id: "c", name: "本地", driver, endpoint: { kind: "file", path: "/tmp/a.db" }, savePassword: false }
    : {
        id: "c",
        name: "本地",
        driver,
        endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root" },
        savePassword: true,
      };
}

async function renderDesigner(driver: DriverKind = "mysql", tab?: TableDesignTab) {
  const backend = createMockBackend([connection(driver)]);
  await backend.openConnection("c");
  setBackendForTesting(backend);
  const schema = driver === "postgres" ? "public" : null;
  const database = driver === "sqlite" ? "main" : "shop";
  const target = tab ?? tableDesignTab("c", database, schema, "customers");
  useTabs.setState({ tabs: [target], activeId: target.id, pending: {}, unsaved: {} });
  const user = userEvent.setup();
  render(
    <QueryClientProvider client={createQueryClient()}>
      <TableDesignView tab={target} active />
    </QueryClientProvider>,
  );
  await screen.findAllByRole("textbox", { name: /：字段名$/ });
  return { user, backend };
}

const tabButton = (name: RegExp) => screen.getByRole("tab", { name });

describe("columns", () => {
  it("inserts, moves, keys and deletes the selected column", async () => {
    const { user } = await renderDesigner();
    await user.click(screen.getByRole("textbox", { name: "email：字段名" }));
    await user.click(screen.getByRole("button", { name: "插入" }));
    await user.keyboard("phone");
    const names = () => screen.getAllByRole("textbox", { name: /：字段名$/ }).map((i) => (i as HTMLInputElement).value);
    expect(names().slice(0, 4)).toEqual(["id", "name", "phone", "email"]);

    await user.click(screen.getByRole("button", { name: "下移" }));
    expect(names().slice(2, 4)).toEqual(["email", "phone"]);
    await user.click(screen.getByRole("button", { name: "上移" }));
    expect(names().slice(2, 4)).toEqual(["phone", "email"]);

    await user.click(screen.getByRole("button", { name: "主键" }));
    expect(screen.getByRole("checkbox", { name: "phone：主键" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "phone：非空" })).toBeChecked();
    expect(screen.getByText("9 个字段 · 主键 id、phone")).toBeInTheDocument();
    await user.click(screen.getByRole("checkbox", { name: "phone：主键" }));
    expect(screen.getByText("9 个字段 · 主键 id")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "删除" }));
    expect(names()).not.toContain("phone");
    expect(screen.getByRole("textbox", { name: "email：字段名" })).toBeInTheDocument();
  });

  it("edits defaults, comments and auto-increment, and shows read-only extras", async () => {
    const { user } = await renderDesigner();
    const fallback = screen.getByRole("textbox", { name: "city：默认值" });
    await user.type(fallback, "'成都'");
    await user.type(screen.getByRole("textbox", { name: "city：注释" }), "城市");
    expect(screen.getByRole("row", { name: /city/ })).toHaveClass("is-edited");
    await user.clear(fallback);
    await user.clear(screen.getByRole("textbox", { name: "city：注释" }));
    expect(screen.getByRole("button", { name: /保存/ })).toBeDisabled();

    expect(screen.getByRole("textbox", { name: "id：默认值" })).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: "id：自增" }));
    expect(screen.getByRole("textbox", { name: "id：默认值" })).toBeEnabled();
  });

  it("picks a type from the suggestions with the keyboard or the mouse", async () => {
    const { user } = await renderDesigner();
    const type = screen.getByRole("combobox", { name: "city：类型" });
    await user.click(type);
    await user.keyboard("{ArrowDown}");
    const list = await screen.findByRole("listbox", { name: "city：类型" });
    expect(within(list).getByRole("option", { name: "int" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();

    await user.clear(type);
    await user.type(type, "medium");
    expect(
      within(screen.getByRole("listbox"))
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["mediumtext"]);
    await user.keyboard("{Enter}");
    expect(type).toHaveValue("mediumtext");

    await user.click(screen.getByRole("button", { name: "city：类型：展开建议" }));
    await user.click(within(screen.getByRole("listbox")).getByRole("option", { name: "json" }));
    expect(type).toHaveValue("json");
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowUp}");
    await act(() => window.dispatchEvent(new Event("scroll")));
    await user.tab();
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("PostgreSQL keeps existing columns in place and hides nothing else", async () => {
    const { user } = await renderDesigner("postgres");
    expect(screen.getByText(/不能调整已有字段的顺序/)).toBeInTheDocument();
    await user.click(screen.getByRole("textbox", { name: "email：字段名" }));
    expect(screen.getByRole("button", { name: "上移" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "下移" })).toBeDisabled();
  });

  it("SQLite has no comment column and shows table flags read-only", async () => {
    const { user } = await renderDesigner("sqlite");
    expect(screen.queryByRole("columnheader", { name: "注释" })).not.toBeInTheDocument();
    await user.click(tabButton(/选项/));
    expect(screen.getByText("WITHOUT ROWID")).toHaveClass("is-off");
    expect(screen.queryByRole("textbox", { name: "表注释" })).not.toBeInTheDocument();
  });
});

describe("indexes and foreign keys", () => {
  it("adds an index, picks columns, flips order, uniqueness and type, then removes it", async () => {
    const { user } = await renderDesigner();
    await user.click(tabButton(/索引/));
    expect(screen.getAllByRole("article")).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "新增索引" }));
    const card = screen.getByRole("article", { name: "索引 idx_customers_3" });
    await user.selectOptions(within(card).getByRole("combobox", { name: /添加字段/ }), "col:city");
    await user.selectOptions(within(card).getByRole("combobox", { name: /添加字段/ }), "col:name");
    await user.click(within(card).getByRole("button", { name: "city：改成降序" }));
    expect(within(card).getByRole("button", { name: "city：改成升序" })).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "移除 name" }));
    await user.click(within(card).getByRole("checkbox", { name: "唯一" }));
    await user.selectOptions(within(card).getByRole("combobox", { name: /索引类型/ }), "FULLTEXT");
    await user.clear(within(card).getByRole("textbox", { name: "索引名" }));
    await user.type(within(card).getByRole("textbox", { name: "索引名" }), "idx_city");
    expect(tabButton(/索引/)).toHaveTextContent("3");

    await user.click(tabButton(/SQL 预览/));
    expect(await screen.findByText(/UNIQUE INDEX `idx_city`/)).toBeInTheDocument();
    await user.click(tabButton(/索引/));
    await user.click(screen.getByRole("button", { name: "删除索引 idx_city" }));
    expect(screen.getAllByRole("article")).toHaveLength(2);
  });

  it("maps columns to a referenced table and its columns", async () => {
    const { user } = await renderDesigner();
    await user.click(tabButton(/外键/));
    await user.click(screen.getByRole("button", { name: "新增外键" }));
    const card = screen.getByRole("article", { name: "外键 第 1 个外键" });
    await user.selectOptions(within(card).getByRole("combobox", { name: /的字段：添加字段/ }), "col:id");
    await user.type(within(card).getByRole("combobox", { name: /引用的表/ }), "ord");
    await user.click(within(screen.getByRole("listbox")).getByRole("option", { name: "orders" }));
    const refs = await within(card).findByRole("combobox", { name: /引用的字段：添加字段/ });
    await user.selectOptions(refs, "customer_id");
    await user.selectOptions(within(card).getByRole("combobox", { name: /删除时/ }), "cascade");
    await user.type(within(card).getByRole("textbox", { name: "外键名" }), "fk_x");

    await user.click(tabButton(/SQL 预览/));
    expect(
      await screen.findByText(/ADD FOREIGN KEY \(`id`\) REFERENCES `orders` \(`customer_id`\)/),
    ).toBeInTheDocument();
    await user.click(tabButton(/外键/));
    await user.click(screen.getByRole("button", { name: "删除外键 fk_x" }));
    expect(screen.getByText("这张表还没有外键。")).toBeInTheDocument();
  });

  it("falls back to typing referenced columns for a table it cannot read", async () => {
    const { user } = await renderDesigner();
    await user.click(tabButton(/外键/));
    await user.click(screen.getByRole("button", { name: "新增外键" }));
    const card = screen.getByRole("article", { name: "外键 第 1 个外键" });
    await user.type(within(card).getByRole("combobox", { name: /引用的表/ }), "elsewhere");
    const manual = within(card).getByRole("textbox", { name: /引用的字段/ });
    await user.type(manual, "a, b");
    expect(manual).toHaveValue("a, b");
  });
});

describe("options and SQL", () => {
  it("changes engine and comment, previews, copies the script and navigates tabs by keyboard", async () => {
    const { user } = await renderDesigner();
    await user.click(tabButton(/选项/));
    const engine = screen.getByRole("combobox", { name: "存储引擎" });
    await user.clear(engine);
    await user.type(engine, "MyISAM");
    await user.type(screen.getByRole("textbox", { name: "表注释" }), "客户");
    await user.type(screen.getByRole("textbox", { name: "排序规则" }), "_x");
    await user.clear(screen.getByRole("combobox", { name: "字符集" }));

    tabButton(/选项/).focus();
    await user.keyboard("{ArrowRight}");
    expect(tabButton(/SQL 预览/)).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText(/COMMENT = '客户'/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "复制" }));
    // user-event 自带一个剪贴板替身,读回来核对。
    expect(await navigator.clipboard.readText()).toContain("ALTER TABLE");
    expect(await screen.findByRole("button", { name: "已复制" })).toBeInTheDocument();
    tabButton(/SQL 预览/).focus();
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(tabButton(/外键/)).toHaveAttribute("aria-selected", "true");
  });

  it("shows why SQL cannot be generated yet", async () => {
    const { user } = await renderDesigner("mysql", newTableTab("c", "shop", null));
    await user.click(tabButton(/SQL 预览/));
    expect(await screen.findByText("请填写表名。")).toBeInTheDocument();
  });

  it("explains an unreadable table and retries", async () => {
    const backend = createMockBackend([connection("mysql")]);
    await backend.openConnection("c");
    vi.spyOn(backend, "readTableDesign").mockRejectedValueOnce({ code: "database", message: "找不到表", fields: [] });
    setBackendForTesting(backend);
    const tab = tableDesignTab("c", "shop", null, "gone");
    useTabs.setState({ tabs: [tab], activeId: tab.id });
    const user = userEvent.setup();
    render(
      <QueryClientProvider client={createQueryClient()}>
        <TableDesignView tab={tab} active />
      </QueryClientProvider>,
    );
    expect(await screen.findByText("读取表结构失败")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "id：字段名" })).toBeInTheDocument());
  });
});
