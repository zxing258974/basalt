// 保存的查询(mock 后端):保存成文件、挂进对象树的「查询」分组、双击重开、覆盖同名、
// 没保存的关闭前确认、⌘T 在选中的库上新建、PostgreSQL 按 schema 存。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useTabs } from "../../stores/tabs";
import { findEditor, installLayoutStubs, setSql } from "../../test/editor";
import { renderApp } from "../../test/renderApp";

installLayoutStubs();

const FLOW_TIMEOUT = 15_000;

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findItem = (name: string) => tree().findByRole("treeitem", { name });
const findRow = async (name: string) => (await findItem(name)).firstElementChild as HTMLElement;
const region = (title: string) => within(screen.getByRole("region", { name: `${title}（SQL 查询）` }));

async function openMysql() {
  const app = renderApp();
  await app.user.dblClick(await findRow("开发"));
  await app.user.dblClick(await findRow("本地 MySQL"));
  return app;
}

/** 展开 shop 下的「查询」分组,返回分组节点。 */
async function expandShopQueries(user: ReturnType<typeof renderApp>["user"]) {
  const shop = await findItem("shop");
  if (shop.getAttribute("aria-expanded") !== "true") {
    await user.click(within(shop).getByText("shop"));
    await user.keyboard("{ArrowRight}");
  }
  const group = await within(shop).findByRole("treeitem", { name: "查询" });
  if (group.getAttribute("aria-expanded") !== "true") await user.dblClick(group.firstElementChild as HTMLElement);
  return group;
}

// 整段流程测试在覆盖率统计、多文件并行时会慢很多,放宽单个用例的时限。
describe("saved queries", { timeout: FLOW_TIMEOUT }, () => {
  it("saves a new query under the database and reopens it from the tree", async () => {
    const { user } = await openMysql();
    const group = await expandShopQueries(user);
    expect(within(group).getByRole("treeitem", { name: "每日销售" })).toBeInTheDocument();

    await user.pointer({ keys: "[MouseRight]", target: group.firstElementChild as HTMLElement });
    await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
    const view = await findEditor("查询 1");
    setSql(view, "SELECT * FROM orders;");
    await waitFor(() => expect(screen.getByRole("tab", { name: /查询 1/ })).toHaveTextContent("（未保存）"));

    await user.click(region("查询 1").getByRole("button", { name: "保存" }));
    const dialog = await screen.findByRole("dialog", { name: "保存查询" });
    expect(dialog).toHaveTextContent("保存到 本地 MySQL / shop");
    await user.type(within(dialog).getByLabelText("名称"), "订单清单");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "保存查询" })).not.toBeInTheDocument());

    expect(await screen.findByRole("tab", { name: /订单清单/ })).not.toHaveTextContent("（未保存）");
    const saved = await within(group).findByRole("treeitem", { name: "订单清单" });
    // 保存后编辑器的读屏名字跟着标签页改名。
    expect(await findEditor("订单清单")).toBe(view);

    // 关掉再从树上双击打开:内容还在,已开着的再双击只是切过去。
    await user.click(screen.getByRole("button", { name: "关闭 订单清单" }));
    expect(useTabs.getState().tabs).toHaveLength(0);
    await user.dblClick(saved.firstElementChild as HTMLElement);
    const reopened = await findEditor("订单清单");
    expect(reopened.state.doc.toString()).toBe("SELECT * FROM orders;");
    await user.dblClick(saved.firstElementChild as HTMLElement);
    expect(useTabs.getState().tabs).toHaveLength(1);
    // 关掉后连着双击两次(第一次还在读文件):也只开一个。
    await user.click(screen.getByRole("button", { name: "关闭 订单清单" }));
    await user.dblClick(saved.firstElementChild as HTMLElement);
    await user.dblClick(saved.firstElementChild as HTMLElement);
    await findEditor("订单清单");
    expect(useTabs.getState().tabs).toHaveLength(1);

    // 改了之后 ⌘S 直接存回原文件。
    const current = await findEditor("订单清单");
    setSql(current, "SELECT * FROM orders LIMIT 1;");
    await user.keyboard("{Meta>}s{/Meta}");
    expect(await screen.findByText("已保存到「订单清单」。")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /订单清单/ })).not.toHaveTextContent("（未保存）");
  });

  it("asks before overwriting a query with the same name", async () => {
    const { user } = await openMysql();
    await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
    await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
    setSql(await findEditor("查询 1"), "SELECT 'new';");
    await user.keyboard("{Meta>}s{/Meta}");
    const dialog = await screen.findByRole("dialog", { name: "保存查询" });
    await user.type(within(dialog).getByLabelText("名称"), "每日销售{Enter}");
    expect(await within(dialog).findByText("这里已经有叫「每日销售」的查询了，要覆盖它吗？")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "覆盖" }));
    expect(await screen.findByRole("tab", { name: /每日销售/ })).toBeInTheDocument();
  });

  it("deletes a saved query after confirming", async () => {
    const { user } = await openMysql();
    const group = await expandShopQueries(user);
    const target = within(group).getByRole("treeitem", { name: "大客户" });
    await user.pointer({ keys: "[MouseRight]", target: target.firstElementChild as HTMLElement });
    await user.click(await screen.findByRole("menuitem", { name: /删除/ }));
    const dialog = await screen.findByRole("dialog", { name: "删除查询「大客户」？" });
    await user.click(within(dialog).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(within(group).queryByRole("treeitem", { name: "大客户" })).not.toBeInTheDocument());
  });

  it("asks before closing a query that was never saved", async () => {
    const { user } = await openMysql();
    await user.click(within(await findItem("shop")).getByText("shop"));
    await user.keyboard("{Meta>}t{/Meta}");
    const view = await findEditor("查询 1");
    expect(region("查询 1").getByRole("combobox", { name: "库" })).toHaveValue("shop");
    setSql(view, "DELETE FROM orders;");

    await user.keyboard("{Meta>}w{/Meta}");
    const prompt = await screen.findByRole("dialog", { name: "放弃未保存的查询？" });
    expect(prompt).toHaveTextContent("有 1 个查询还没有保存");
    await user.click(within(prompt).getByRole("button", { name: "取消" }));
    expect(useTabs.getState().tabs).toHaveLength(1);

    setSql(view, "");
    await user.keyboard("{Meta>}w{/Meta}");
    expect(useTabs.getState().tabs).toHaveLength(0);
  });

  it("keeps the query tab, and its SQL, when the connection is deleted", async () => {
    const { user } = await openMysql();
    await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
    await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
    setSql(await findEditor("查询 1"), "SELECT 'keep me';");
    await user.pointer({ keys: "[MouseRight]", target: await findRow("本地 MySQL") });
    await user.click(await screen.findByRole("menuitem", { name: "删除连接…" }));
    const dialog = await screen.findByRole("dialog", { name: "删除连接「本地 MySQL」?" });
    await user.click(within(dialog).getByRole("button", { name: "删除" }));
    expect(await screen.findByText(/这个连接已经删除了/)).toBeInTheDocument();
    expect((await findEditor("查询 1")).state.doc.toString()).toBe("SELECT 'keep me';");
    expect(region("查询 1").getByRole("button", { name: /^执行/ })).toBeDisabled();
  });

  it("keeps PostgreSQL queries under the schema", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("FOLIO 测试库"));
    const prompt = await screen.findByRole("dialog", { name: "连接到「FOLIO 测试库」" });
    await user.type(within(prompt).getByLabelText("密码"), "folio");
    await user.click(within(prompt).getByRole("button", { name: "连接" }));

    await user.dblClick(await findRow("folio"));
    await user.dblClick(await findRow("diku_mod_inventory_storage"));
    const schema = await findItem("diku_mod_inventory_storage");
    const group = await within(schema).findByRole("treeitem", { name: "查询" });
    await user.dblClick(group.firstElementChild as HTMLElement);
    await user.dblClick(await within(group).findByText("馆藏统计"));
    const view = await findEditor("馆藏统计");
    expect(view.state.doc.toString()).toBe("SELECT count(*) AS n FROM item;");
    expect(region("馆藏统计").getByRole("combobox", { name: "schema" })).toHaveValue("diku_mod_inventory_storage");

    // 从库节点新建的查询还没选 schema:保存前要先选。
    await user.pointer({ keys: "[MouseRight]", target: await findRow("folio") });
    await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
    setSql(await findEditor("查询 1"), "SELECT 1;");
    await user.click(region("查询 1").getByRole("button", { name: "保存" }));
    expect(await screen.findByText(/先在上面选一个 schema/)).toBeInTheDocument();
    await user.selectOptions(region("查询 1").getByRole("combobox", { name: "schema" }), "public");
    await user.click(region("查询 1").getByRole("button", { name: "保存" }));
    expect(await screen.findByRole("dialog", { name: "保存查询" })).toHaveTextContent(
      "保存到 FOLIO 测试库 / folio / public",
    );
  });
});
