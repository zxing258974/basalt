// Redis 命令行的交互流程:执行、换库和事务跟着提示符走、危险命令先确认、停下正在等的命令、
// 不支持的命令、历史与补全、带密码的命令不回显不记、关标签页断开连接。
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import { redisConsoleTab, type RedisConsoleTab } from "../../stores/tabs";
import { RedisConsoleView } from "./RedisConsoleView";

const CONNECTION = "mock-redis";

beforeEach(() => localStorage.clear());
afterEach(() => setBackendForTesting(null));

async function renderConsole(db = 0) {
  const backend = createMockBackend();
  await backend.openConnection(CONNECTION);
  setBackendForTesting(backend);
  const tab: RedisConsoleTab = redisConsoleTab(CONNECTION, db, []);
  const view = render(
    <QueryClientProvider client={createQueryClient()}>
      <RedisConsoleView tab={tab} active />
    </QueryClientProvider>,
  );
  const input = await screen.findByRole("textbox", { name: "Redis 命令" });
  return { backend, tab, view, input, user: userEvent.setup() };
}

const log = () => screen.getByRole("log", { name: "命令行输出" });
/** 最后一条命令的输出区。 */
const lastEntry = () => within(log()).getAllByRole("region").at(-1)!;

describe("redis console", () => {
  it("runs commands and prints replies like redis-cli", async () => {
    const { input, user } = await renderConsole();
    expect(input).toHaveFocus();
    await user.type(input, "HGETALL user:1001{Enter}");
    const entry = await within(log()).findByRole("region", { name: "HGETALL user:1001" });
    await waitFor(() => expect(entry).toHaveTextContent('1) "name"2) "黄勇"'));
    expect(within(entry).getByText("127.0.0.1:6379>")).toBeInTheDocument();
    expect(input).toHaveValue("");

    await user.type(input, "incr counter:visits{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("(integer) 1048577"));
    await user.type(input, "nosuch thing{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("(error) ERR unknown command 'nosuch'"));
    await user.type(input, 'set k "open{Enter}');
    await waitFor(() => expect(lastEntry()).toHaveTextContent("引号没有配对"));
  });

  it("follows SELECT and MULTI in the prompt", async () => {
    const { input, user } = await renderConsole();
    await user.type(input, "select 1{Enter}");
    await waitFor(() => expect(screen.getByText("db1")).toBeInTheDocument());
    await user.type(input, "multi{Enter}");
    expect(await screen.findByText("事务中")).toBeInTheDocument();
    await user.type(input, "llen queue:email{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("127.0.0.1:6379[1](TX)>llen queue:email"));
    expect(lastEntry()).toHaveTextContent("QUEUED");
    await user.type(input, "exec{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("1) (integer) 3"));
    expect(screen.queryByText("事务中")).not.toBeInTheDocument();
  });

  it("asks before a dangerous command and does nothing when declined", async () => {
    const { backend, input, user } = await renderConsole(1);
    await user.type(input, "flushdb{Enter}");
    const dialog = await screen.findByRole("dialog", { name: "执行 FLUSHDB？" });
    expect(dialog).toHaveTextContent("删掉当前库里的全部键");
    expect(lastEntry()).toHaveTextContent("等你确认…");
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(lastEntry()).toHaveTextContent("没有执行。"));
    expect(input).toHaveFocus();
    expect((await backend.redisKeyspace(CONNECTION)).databases[1]!.keys).toBe(3);

    await user.type(input, "flushdb{Enter}");
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "执行" }));
    await waitFor(() => expect(lastEntry()).toHaveTextContent("OK"));
    expect((await backend.redisKeyspace(CONNECTION)).databases[1]!.keys).toBe(0);
  });

  it("stops a blocked command and reconnects for the next one", async () => {
    const { input, user } = await renderConsole();
    await user.type(input, "select 1{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("OK"));
    await user.type(input, "blpop nothing 0{Enter}");
    expect(await within(lastEntry()).findByText("正在等回复…")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("已停下"));

    // 重新连上时回到界面上显示的库(db1 的队列里有 3 个键)。
    await user.type(input, "dbsize{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("(integer) 3"));
  });

  it("stops the rest of a pasted batch", async () => {
    const { backend, input, user } = await renderConsole();
    input.focus();
    await user.paste("blpop nothing 0\nset after 1");
    await user.keyboard("{Enter}");
    expect(await within(lastEntry()).findByText("正在等回复…")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "停止" }));
    await waitFor(() => expect(lastEntry()).toHaveTextContent("已停下"));
    expect(within(log()).getAllByRole("region")).toHaveLength(1);
    expect(await backend.redisConsoleRun(CONNECTION, "check", 0, "exists after", false)).toMatchObject({
      reply: { kind: "int", value: "0" },
    });
    await waitFor(() => expect(input).toHaveFocus());
  });

  it("runs nothing more after the tab is closed", async () => {
    const { backend, input, user, view } = await renderConsole();
    input.focus();
    await user.paste("blpop nothing 0\nset after 1");
    await user.keyboard("{Enter}");
    await within(lastEntry()).findByText("正在等回复…");
    view.unmount();
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(await backend.redisConsoleRun(CONNECTION, "check", 0, "exists after", false)).toMatchObject({
      reply: { kind: "int", value: "0" },
    });
  });

  it("waits to ask about a dangerous command until its tab is shown", async () => {
    const { input, user, view, tab } = await renderConsole(1);
    await user.type(input, "flushdb");
    view.rerender(
      <QueryClientProvider client={createQueryClient()}>
        <RedisConsoleView tab={tab} active={false} />
      </QueryClientProvider>,
    );
    await user.keyboard("{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("等你确认…"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    view.rerender(
      <QueryClientProvider client={createQueryClient()}>
        <RedisConsoleView tab={tab} active />
      </QueryClientProvider>,
    );
    expect(await screen.findByRole("dialog", { name: "执行 FLUSHDB？" })).toBeInTheDocument();
  });

  it("refuses subscriptions with a reason", async () => {
    const { input, user } = await renderConsole();
    await user.type(input, "subscribe news{Enter}");
    expect(await within(log()).findByRole("alert")).toHaveTextContent("命令行暂不支持订阅");
  });

  it("recalls history with the arrow keys and completes command names", async () => {
    const { input, user } = await renderConsole();
    await user.type(input, "get a{Enter}");
    await user.type(input, "get b{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("(nil)"));
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveValue("get b");
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveValue("get a");
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(input).toHaveValue("");

    await user.type(input, "hgeta");
    const list = screen.getByRole("listbox", { name: "命令补全" });
    expect(
      within(list)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual([expect.stringMatching(/^HGETALL/)]);
    await user.keyboard("{Tab}");
    expect(input).toHaveValue("HGETALL ");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(screen.getByText("HGETALL key")).toBeInTheDocument();
  });

  it("hides passwords and keeps them out of the history", async () => {
    const { input, user } = await renderConsole();
    await user.type(input, "auth app hunter2{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveAccessibleName("AUTH ••••••"));
    expect(log()).not.toHaveTextContent("hunter2");
    expect(JSON.stringify(localStorage)).not.toContain("hunter2");
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveValue("");
  });

  it("runs pasted lines in order and clears the screen", async () => {
    const { input, user } = await renderConsole();
    input.focus();
    await user.paste("set pasted 1\nincr pasted");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(lastEntry()).toHaveTextContent("(integer) 2"));
    expect(within(log()).getAllByRole("region")).toHaveLength(2);
    await user.keyboard("{Meta>}k{/Meta}");
    expect(within(log()).queryAllByRole("region")).toHaveLength(0);
  });

  it("closes its own connection when the tab goes away", async () => {
    const { backend, view, tab } = await renderConsole();
    const close = vi.spyOn(backend, "redisConsoleClose");
    view.unmount();
    await waitFor(() => expect(close).toHaveBeenCalledWith(CONNECTION, tab.id));
  });
});
