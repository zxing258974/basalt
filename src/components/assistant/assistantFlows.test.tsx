// SQL 助手(mock 后端):生成 SQL 并插入编辑器(不执行)、优化当前 SQL、停止、出错、⌘↩ 不会执行 SQL、没配置模型时。
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import type { ChatDelta, ChatRequest } from "../../lib/llmTypes";
import { createMockBackend } from "../../lib/mockBackend";
import { findEditor, installLayoutStubs, setSql } from "../../test/editor";
import { useTabs } from "../../stores/tabs";
import { renderApp } from "../../test/renderApp";

installLayoutStubs();

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;

/** 在 shop 上新建查询,打开 SQL 助手。 */
async function openAssistant(backend: Backend = createMockBackend()) {
  const app = renderApp(backend);
  const { user } = app;
  await user.dblClick(await findRow("开发"));
  await user.dblClick(await findRow("本地 MySQL"));
  await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
  await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
  const editor = await findEditor("查询 1");
  await user.click(screen.getByRole("button", { name: "助手" }));
  const panel = within(screen.getByRole("complementary", { name: "SQL 助手" }));
  return { ...app, editor, panel };
}

/** 由测试控制节奏的对话:推出第一段后停住,直到被取消。 */
function pausedChat(backend: Backend) {
  const listeners = new Set<(delta: ChatDelta) => void>();
  const requests: ChatRequest[] = [];
  let stop: (() => void) | null = null;
  const controlled: Backend = {
    ...backend,
    onLlmChat: (handler) => {
      listeners.add(handler);
      return Promise.resolve(() => void listeners.delete(handler));
    },
    llmChat: (requestId, request) => {
      requests.push(request);
      return new Promise((_resolve, reject) => {
        listeners.forEach((l) => l({ requestId, text: "先把函数去掉：\n```sql\nSELECT" }));
        stop = () => reject(Object.assign(new Error("已取消"), { code: "cancelled", fields: [] }));
      });
    },
    cancelLlmChat: vi.fn(() => {
      stop?.();
      return Promise.resolve(true);
    }),
  };
  return { controlled, requests };
}

describe("SQL assistant", { timeout: 15_000 }, () => {
  it("streams an answer with the schema attached and inserts SQL without running it", async () => {
    const backend = createMockBackend();
    const chat = vi.spyOn(backend, "llmChat");
    const run = vi.spyOn(backend, "runStatement");
    const { user, editor, panel } = await openAssistant(backend);
    expect(await panel.findByText(/会附带「shop」里 5 张表的结构/)).toBeInTheDocument();
    expect(panel.getByText(/助手只写 SQL，不会执行/)).toBeInTheDocument();

    await user.type(panel.getByRole("textbox", { name: "问助手" }), "最近 30 天的订单");
    await user.click(panel.getByRole("button", { name: "生成 SQL" }));
    const insert = await panel.findByRole("button", { name: "插入到编辑器" });
    expect(panel.getByText(/用 customer_id 关联 customers/)).toBeInTheDocument();
    expect(panel.getByRole("textbox", { name: "问助手" })).toHaveValue("");

    const request = chat.mock.calls[0]![1];
    expect(request.modelId).toBe("mock-deepseek");
    expect(request.system).toContain("MySQL 8.4.3，当前库是「shop」");
    expect(request.system).toMatch(/customers\(id int 主键/);
    expect(request.messages).toEqual([{ role: "user", content: "最近 30 天的订单" }]);

    await user.click(insert);
    expect(editor.state.doc.toString()).toContain("JOIN customers c ON c.id = o.customer_id");
    expect(run).not.toHaveBeenCalled();

    // 追问:带上前面这一问一答。
    await user.type(panel.getByRole("textbox", { name: "问助手" }), "只要已付款的");
    await user.click(panel.getByRole("button", { name: "生成 SQL" }));
    await waitFor(() => expect(chat).toHaveBeenCalledTimes(2));
    const followUp = chat.mock.calls[1]![1];
    expect(followUp.messages.map((m) => m.role)).toEqual(["user", "assistant", "user"]);
    await waitFor(() => expect(panel.getAllByRole("button", { name: "插入到编辑器" })).toHaveLength(2));

    await user.click(panel.getByRole("button", { name: "新对话" }));
    expect(panel.queryByRole("list", { name: "对话" })).not.toBeInTheDocument();
  });

  it("optimizes the selected SQL and can be stopped midway", async () => {
    const { controlled, requests } = pausedChat(createMockBackend());
    const { user, editor, panel } = await openAssistant(controlled);
    await user.click(panel.getByRole("button", { name: "优化当前 SQL" }));
    expect(panel.getByRole("status")).toHaveTextContent("编辑器里还没有 SQL");
    expect(requests).toEqual([]);

    setSql(editor, "SELECT 1;\nSELECT * FROM orders WHERE DATE(created_at) = '2026-10-01';", { from: 10, to: 69 });
    await user.click(panel.getByRole("button", { name: "优化当前 SQL" }));
    const question = requests[0]!.messages.at(-1)!.content;
    expect(question).toContain("请优化下面的 SQL");
    expect(question).toContain("WHERE DATE(created_at)");
    expect(question).not.toContain("SELECT 1;");

    // 流到一半:代码块还没收尾,不给插入按钮。
    expect(await panel.findByText("先把函数去掉：")).toBeInTheDocument();
    expect(panel.getByText("SELECT")).toBeInTheDocument();
    expect(panel.queryByRole("button", { name: "插入到编辑器" })).not.toBeInTheDocument();
    expect(panel.getByRole("combobox", { name: "模型" })).toBeDisabled();
    await user.click(panel.getByRole("button", { name: "停止" }));
    expect(controlled.cancelLlmChat).toHaveBeenCalledTimes(1);
    expect(await panel.findByText("已停止。")).toBeInTheDocument();
    expect(panel.getByRole("button", { name: "生成 SQL" })).toBeInTheDocument();
  });

  it("sends with ⌘↩ instead of running the editor's SQL, and shows errors", async () => {
    const backend = createMockBackend();
    const run = vi.spyOn(backend, "runStatement");
    vi.spyOn(backend, "llmChat").mockRejectedValue({
      code: "llm",
      message: "模型请求失败：HTTP 401，请检查 API Key：invalid api key",
      fields: [],
    });
    const { user, editor, panel } = await openAssistant(backend);
    setSql(editor, "SELECT 1;");
    const input = panel.getByRole("textbox", { name: "问助手" });
    await user.type(input, "数一下订单");
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    expect(await panel.findByRole("alert")).toHaveTextContent("请检查 API Key");
    expect(run).not.toHaveBeenCalled();
  });

  it("asks to configure a model first when there is none", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "listLlmModels").mockResolvedValue({ models: [], active: null });
    const { user, panel } = await openAssistant(backend);
    expect(await panel.findByText(/还没有配置模型/)).toBeInTheDocument();
    await user.click(panel.getByRole("button", { name: "配置模型…" }));
    expect(screen.getByRole("dialog", { name: "模型设置" })).toBeInTheDocument();
  });

  it("keeps the conversation when the panel is closed and reopened", async () => {
    const { user, panel } = await openAssistant();
    await user.type(panel.getByRole("textbox", { name: "问助手" }), "订单");
    await user.click(panel.getByRole("button", { name: "生成 SQL" }));
    await panel.findByRole("button", { name: "插入到编辑器" });
    await user.click(panel.getByRole("button", { name: "关闭 SQL 助手" }));
    expect(screen.queryByRole("complementary", { name: "SQL 助手" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "历史" }));
    await user.click(screen.getByRole("button", { name: "助手" }));
    expect(screen.queryByRole("complementary", { name: "执行历史" })).not.toBeInTheDocument();
    const again = within(screen.getByRole("complementary", { name: "SQL 助手" }));
    expect(again.getByRole("button", { name: "插入到编辑器" })).toBeInTheDocument();
  });

  it("settles with the full text even if the last pieces arrive late, and flags risky SQL", async () => {
    const backend = createMockBackend();
    const full = "删掉这一单：\n\n```sql\nDELETE FROM orders WHERE id = 7;\n```";
    const listeners = new Set<(delta: ChatDelta) => void>();
    const controlled: Backend = {
      ...backend,
      onLlmChat: (handler) => {
        listeners.add(handler);
        return Promise.resolve(() => void listeners.delete(handler));
      },
      // 只推了前半段,命令就返回了:后半段的事件「还在路上」。
      llmChat: (requestId) => {
        listeners.forEach((l) => l({ requestId, text: full.slice(0, 20) }));
        return Promise.resolve({ chars: full.length, text: full });
      },
    };
    const { user, panel } = await openAssistant(controlled);
    await user.type(panel.getByRole("textbox", { name: "问助手" }), "删掉 7 号订单");
    await user.click(panel.getByRole("button", { name: "生成 SQL" }));
    expect(await panel.findByRole("button", { name: "插入到编辑器" })).toBeInTheDocument();
    expect(panel.getByText("DELETE FROM orders WHERE id = 7;")).toBeInTheDocument();
    expect(panel.getByText(/含 DELETE：会改动数据或表结构/)).toBeInTheDocument();
    expect(panel.getByRole("textbox", { name: "问助手" })).toHaveFocus();
  });

  it("is resized from its left edge, separately from the history panel", async () => {
    localStorage.clear();
    HTMLElement.prototype.setPointerCapture = vi.fn();
    const { user, container } = await openAssistant();
    const body = container.querySelector<HTMLElement>(".query-body")!;
    const drag = (name: string, dx: number) => {
      const resizer = screen.getByRole("separator", { name });
      fireEvent.pointerDown(resizer, { clientX: 800, pointerId: 1 });
      fireEvent.pointerMove(resizer, { clientX: 800 + dx });
      fireEvent.pointerUp(resizer);
    };
    expect(body.style.getPropertyValue("--query-side-width")).toBe("340px");
    drag("调整 SQL 助手宽度", -160);
    expect(body.style.getPropertyValue("--query-side-width")).toBe("500px");
    expect(localStorage.getItem("basalt.assistantWidth")).toBe("500");

    await user.click(screen.getByRole("button", { name: "历史" }));
    expect(body.style.getPropertyValue("--query-side-width")).toBe("300px");
    drag("调整执行历史宽度", 40);
    expect(body.style.getPropertyValue("--query-side-width")).toBe("260px");
    await user.click(screen.getByRole("button", { name: "助手" }));
    expect(body.style.getPropertyValue("--query-side-width")).toBe("500px");
  });

  it("can still stop after the panel is reopened, and closing the tab stops the answer", async () => {
    const { controlled } = pausedChat(createMockBackend());
    const { user, panel } = await openAssistant(controlled);
    await user.type(panel.getByRole("textbox", { name: "问助手" }), "订单");
    await user.click(panel.getByRole("button", { name: "生成 SQL" }));
    await panel.findByText("先把函数去掉：");
    await user.click(panel.getByRole("button", { name: "关闭 SQL 助手" }));
    await user.click(screen.getByRole("button", { name: "助手" }));
    const again = within(screen.getByRole("complementary", { name: "SQL 助手" }));
    await user.click(again.getByRole("button", { name: "停止" }));
    expect(controlled.cancelLlmChat).toHaveBeenCalledTimes(1);
    expect(await again.findByText("已停止。")).toBeInTheDocument();

    await user.type(again.getByRole("textbox", { name: "问助手" }), "再来");
    await user.click(again.getByRole("button", { name: "生成 SQL" }));
    await waitFor(() => expect(again.getAllByText("先把函数去掉：")).toHaveLength(2));
    act(() => useTabs.getState().close(useTabs.getState().activeId!));
    await waitFor(() => expect(controlled.cancelLlmChat).toHaveBeenCalledTimes(2));
  });
});
