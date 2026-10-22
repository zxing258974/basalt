// 查询标签页的执行流程(mock 后端):全部执行、只执行当前或选中的、出错就停、取消、USE 跟着换库、执行历史。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useTabs } from "../../stores/tabs";
import { findEditor, installLayoutStubs, setSql } from "../../test/editor";
import { renderApp } from "../../test/renderApp";

installLayoutStubs();

const FLOW_TIMEOUT = 15_000;

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;

/** 打开本地 MySQL,在 shop 上右键「新建查询」。 */
async function openShopQuery() {
  const app = renderApp();
  const { user } = app;
  await user.dblClick(await findRow("开发"));
  await user.dblClick(await findRow("本地 MySQL"));
  await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
  await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
  const view = await findEditor("查询 1");
  return { ...app, view };
}

const header = () => within(screen.getByRole("region", { name: "查询 1（SQL 查询）" }));
const messages = () => screen.findByRole("log", { name: "执行消息" });

// 整段流程测试在覆盖率统计、多文件并行时会慢很多,放宽单个用例的时限。
describe("running statements", { timeout: FLOW_TIMEOUT }, () => {
  it("runs everything in order: one tab per result set, then the messages", async () => {
    const { user, view } = await openShopQuery();
    setSql(view, "SELECT * FROM customers LIMIT 5;\nUPDATE orders SET status = 'paid';\nSELECT 1 AS one;");
    await user.click(header().getByRole("button", { name: "全部执行" }));

    const first = await screen.findByRole("tab", { name: /结果 1/ });
    expect(first).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /结果 2/ })).toBeInTheDocument();
    const grid = await screen.findByRole("grid", { name: "查询 1 的结果 1" });
    await waitFor(() => expect(within(grid).getAllByRole("row")).toHaveLength(6));

    await user.click(screen.getByRole("tab", { name: "消息" }));
    const log = await messages();
    expect(within(log).getByText("3 条语句执行成功，用时 18 ms。")).toBeInTheDocument();
    expect(within(log).getByText("影响 3 行")).toBeInTheDocument();
    expect(within(log).getByText("5 行")).toBeInTheDocument();
  });

  it("sorts a result through the backend and only offers copying when it can't be edited", async () => {
    const { user, view } = await openShopQuery();
    setSql(view, "SELECT * FROM (SELECT * FROM customers) c LIMIT 5");
    await user.click(header().getByRole("button", { name: /^执行/ }));
    const grid = await screen.findByRole("grid", { name: "查询 1 的结果 1" });
    const firstId = () => within(within(grid).getAllByRole("row")[1]!).getAllByRole("gridcell")[0]!.textContent;
    await waitFor(() => expect(firstId()).toBe("1"));
    await user.click(within(grid).getByRole("button", { name: /^id/ }));
    await user.click(within(grid).getByRole("button", { name: /^id/ }));
    await waitFor(() => expect(firstId()).toBe("5"));
    expect(within(grid).getByRole("columnheader", { name: /^id/ })).toHaveAttribute("aria-sort", "descending");
    expect(screen.getByText("结果来自子查询或 UNION，不能在这里改。")).toBeInTheDocument();

    await user.pointer({
      keys: "[MouseRight]",
      target: within(within(grid).getAllByRole("row")[1]!).getAllByRole("gridcell")[1]!,
    });
    const menu = await screen.findByRole("menu");
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((item) => item.textContent),
    ).toEqual(["复制⌘C"]);
  });

  it("marks where a statement went wrong", async () => {
    const { user, view } = await openShopQuery();
    setSql(view, "SELECT 1 AS ok;\nSELECT * FROM missing;");
    await user.click(header().getByRole("button", { name: "全部执行" }));
    await screen.findByText(/第 2 条出错/);
    const mark = view.contentDOM.querySelector(".cm-sql-error");
    expect(mark?.textContent).toBe("missing");
    expect(mark?.getAttribute("title")).toContain("doesn't exist");

    // 改到标出来的地方,标记就去掉。
    view.dispatch({ changes: { from: view.state.doc.length - 2, insert: "x" } });
    expect(view.contentDOM.querySelector(".cm-sql-error")).toBeNull();
  });

  it("lists what PostgreSQL said while running a statement", async () => {
    const { user } = renderApp();
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("FOLIO 测试库"));
    const prompt = await screen.findByRole("dialog", { name: "连接到「FOLIO 测试库」" });
    await user.type(within(prompt).getByLabelText("密码"), "folio");
    await user.click(within(prompt).getByRole("button", { name: "连接" }));
    await user.pointer({ keys: "[MouseRight]", target: await findRow("folio") });
    await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
    const view = await findEditor("查询 1");

    setSql(view, "DO $$ BEGIN RAISE NOTICE '处理了 3 行'; RAISE WARNING '跳过 1 行'; END $$;");
    await user.click(header().getByRole("button", { name: "全部执行" }));
    const server = within(await messages()).getByRole("list", { name: "服务器消息" });
    expect(
      within(server)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["NOTICE处理了 3 行", "WARNING跳过 1 行"]);
  });

  it("runs only the statement under the cursor, or everything selected", async () => {
    const { user, view } = await openShopQuery();
    const text = "SELECT 1 AS a;\nSELECT 2 AS b;\nSELECT 3 AS c;";
    setSql(view, text, text.indexOf("2"));
    await user.click(header().getByRole("button", { name: /^执行/ }));
    await screen.findByRole("tab", { name: /结果 1/ });
    expect(screen.queryByRole("tab", { name: /结果 2/ })).not.toBeInTheDocument();
    expect(await screen.findByRole("columnheader", { name: /^b/ })).toBeInTheDocument();

    setSql(view, text, { from: text.indexOf("SELECT 2"), to: text.length });
    await user.click(header().getByRole("button", { name: /^执行/ }));
    expect(await screen.findByRole("tab", { name: /结果 2/ })).toBeInTheDocument();
  });

  it("says so when there is nothing to run", async () => {
    const { user, view } = await openShopQuery();
    setSql(view, "-- 只有注释");
    await user.click(header().getByRole("button", { name: /^执行/ }));
    expect(await screen.findByText("光标所在的位置没有语句。")).toBeInTheDocument();
    await user.click(header().getByRole("button", { name: "全部执行" }));
    expect(await screen.findByText("编辑器里没有可执行的语句。")).toBeInTheDocument();
  });

  it("stops at the first error and opens the messages", async () => {
    const { user, view } = await openShopQuery();
    setSql(view, "SELECT * FROM missing;\nSELECT 1;");
    await user.click(header().getByRole("button", { name: "全部执行" }));
    const log = await messages();
    expect(await within(log).findByText("第 1 条出错，后面 1 条没有执行。")).toBeInTheDocument();
    expect(within(log).getByText(/^1146 - Table 'shop.missing'/)).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "消息" })).toHaveAttribute("aria-selected", "true");
  });

  it("cancels a long statement and skips the rest", async () => {
    const { user, view } = await openShopQuery();
    setSql(view, "SELECT SLEEP(30);\nSELECT 1;");
    await user.click(header().getByRole("button", { name: "全部执行" }));
    const cancel = header().getByRole("button", { name: "取消" });
    await waitFor(() => expect(cancel).toBeEnabled());
    expect(await within(await messages()).findByText("执行中…")).toBeInTheDocument();
    await user.click(cancel);
    const log = await messages();
    expect(await within(log).findByText("已取消：执行了 1 条，后面 1 条没有执行。")).toBeInTheDocument();
    expect(within(log).getByText("已取消：Query execution was interrupted")).toBeInTheDocument();
    expect(header().getByRole("button", { name: "全部执行" })).toBeEnabled();
  });

  it("follows USE to another database", async () => {
    const { user, view } = await openShopQuery();
    expect(header().getByRole("combobox", { name: "库" })).toHaveValue("shop");
    setSql(view, "USE mysql");
    await user.click(header().getByRole("button", { name: /^执行/ }));
    await waitFor(() => expect(header().getByRole("combobox", { name: "库" })).toHaveValue("mysql"));
    const tab = useTabs.getState().tabs[0];
    expect(tab?.kind === "query" && tab.scope.database).toBe("mysql");

    await user.selectOptions(header().getByRole("combobox", { name: "库" }), "information_schema");
    const after = useTabs.getState().tabs[0];
    expect(after?.kind === "query" && after.scope.database).toBe("information_schema");
  });
});

// 整段流程测试在覆盖率统计、多文件并行时会慢很多,放宽单个用例的时限。
describe("history", { timeout: FLOW_TIMEOUT }, () => {
  it("lists executed statements newest first and fills the editor on click", async () => {
    const { user, view } = await openShopQuery();
    setSql(view, "SELECT 42 AS answer;");
    await user.click(header().getByRole("button", { name: /^执行/ }));
    await screen.findByRole("tab", { name: /结果 1/ });

    await user.click(header().getByRole("button", { name: "历史" }));
    const panel = await screen.findByRole("complementary", { name: "执行历史" });
    const entry = await within(panel).findByRole("button", { name: /SELECT 42 AS answer/ });
    setSql(view, "");
    await user.click(entry);
    expect(view.state.doc.toString()).toBe("SELECT 42 AS answer");

    await user.type(within(panel).getByRole("searchbox", { name: "搜索执行历史" }), "nothing-like-this");
    expect(await within(panel).findByText("没有找到")).toBeInTheDocument();
    await user.clear(within(panel).getByRole("searchbox", { name: "搜索执行历史" }));
    await within(panel).findByRole("button", { name: /SELECT 42 AS answer/ });

    await user.click(within(panel).getByRole("button", { name: "清空" }));
    await user.click(within(panel).getByRole("button", { name: "确定清空" }));
    expect(await within(panel).findByText("还没有执行过语句")).toBeInTheDocument();
    await user.click(within(panel).getByRole("button", { name: "关闭执行历史" }));
    expect(screen.queryByRole("complementary", { name: "执行历史" })).not.toBeInTheDocument();
  });
});
