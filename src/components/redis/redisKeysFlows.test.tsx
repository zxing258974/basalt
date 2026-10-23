// Redis 键浏览的交互流程:分组与筛选、表格改 hash → ⌘S 预览命令 → 提交、被别人改过就整批不写、
// 字符串编辑、过期时间 / 改名 / 删除 / 新建、换键前确认放弃修改。
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { RowChange } from "../../lib/types";
import { redisKeysTab, useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { DiscardChangesDialog } from "../data/DiscardChangesDialog";
import { RedisKeysView } from "./RedisKeysView";

const CONNECTION = "mock-redis";

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

async function renderKeys() {
  const backend = createMockBackend();
  await backend.openConnection(CONNECTION);
  setBackendForTesting(backend);
  const tab = redisKeysTab(CONNECTION, 0);
  useTabs.setState({ tabs: [tab], activeId: tab.id, pending: {} });
  useUi.setState({ discardPrompt: null });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <RedisKeysView tab={tab} active />
      <DiscardChangesDialog />
    </QueryClientProvider>,
  );
  const tree = await screen.findByRole("tree", { name: "db0 的键" });
  await within(tree).findByRole("treeitem", { name: /^user，/ });
  return { backend, tree, user: userEvent.setup(), tabId: tab.id };
}

const escaped = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const folder = (tree: HTMLElement, name: string) =>
  within(tree).getByRole("treeitem", { name: new RegExp(`^${escaped(name)}，`) });
const keyRow = (tree: HTMLElement, name: string) =>
  within(tree).findByRole("treeitem", { name: new RegExp(`^${escaped(name)}（`) });

/** 一层层展开分组,点开里面的键,等右边显示出它的名字。 */
async function openKey(tree: HTMLElement, user: UserEvent, groups: string | string[], name: string) {
  for (const group of [groups].flat()) await user.click(folder(tree, group));
  await user.click(await keyRow(tree, name));
  return screen.findByRole("heading", { name });
}

/** 值表格里 field 是 `field` 的那一行的 value 格。 */
function valueCell(grid: HTMLElement, field: string): HTMLElement {
  const row = within(grid)
    .getAllByRole("row")
    .find((r) => within(r).queryAllByRole("gridcell")[0]?.textContent === field);
  return within(row!).getAllByRole("gridcell")[1]!;
}

async function editHashValue(user: UserEvent, key: string, field: string, value: string) {
  const grid = await screen.findByRole("grid", { name: `${key} 的值` });
  await user.dblClick(valueCell(grid, field));
  const editor = screen.getByRole("textbox", { name: "编辑「value」" });
  await user.clear(editor);
  await user.type(editor, `${value}{Enter}`);
  return grid;
}

const hashValue = async (backend: ReturnType<typeof createMockBackend>, key: string, field: string) => {
  const detail = await backend.redisReadKey(CONNECTION, 0, `s:${key}`);
  if (detail.value.kind !== "table") throw new Error("不是表格值");
  const row = detail.value.page.rows.find((r) => r[0]?.kind === "text" && r[0].value === field);
  return row?.[1];
};

describe("redis keys", () => {
  it("groups keys by colon, filters by name and by type", async () => {
    const { tree, user } = await renderKeys();
    expect(folder(tree, "product")).toHaveAttribute("aria-expanded", "false");
    expect(within(tree).queryByRole("treeitem", { name: /^product:2001（/ })).not.toBeInTheDocument();
    expect(screen.getByText(/已列出 57 个 \/ 共 57 个/)).toBeInTheDocument();

    await user.type(screen.getByRole("textbox", { name: "按名字筛选键" }), "product:200?{Enter}");
    // 筛选时分组全展开,匹配到的键直接露出来。
    expect(await keyRow(tree, "product:2009")).toBeInTheDocument();
    await waitFor(() => expect(within(tree).queryByRole("treeitem", { name: /^user，/ })).not.toBeInTheDocument());
    expect(within(tree).getAllByRole("treeitem", { name: /（string）$/ })).toHaveLength(9);

    await user.click(screen.getByRole("button", { name: "清除筛选" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "按类型筛选" }), "zset");
    expect(await keyRow(tree, "leaderboard:weekly")).toBeInTheDocument();
    await waitFor(() => expect(within(tree).getAllByRole("treeitem", { name: /（/ })).toHaveLength(1));

    await user.click(screen.getByRole("button", { name: "平铺" }));
    expect(within(tree).getByRole("treeitem", { name: "leaderboard:weekly（zset）" })).toHaveAttribute(
      "aria-level",
      "1",
    );
  });

  it("walks the key list with the arrow keys", async () => {
    const { tree, user } = await renderKeys();
    tree.focus();
    await user.keyboard("{Enter}");
    expect(folder(tree, "config")).toHaveAttribute("aria-expanded", "true");
    await user.keyboard("{ArrowDown}");
    expect(await screen.findByRole("heading", { name: "config:feature_flags" })).toBeInTheDocument();
    expect(await keyRow(tree, "config:feature_flags")).toHaveAttribute("aria-selected", "true");
  });

  it("edits a hash field, previews HSET with ⌘S and commits it", async () => {
    const { backend, tree, user } = await renderKeys();
    await openKey(tree, user, "user", "user:1001");
    const grid = await editHashValue(user, "user:1001", "city", "苏州 西园");
    expect(valueCell(grid, "city")).toHaveClass("is-edited");
    expect(screen.getByText("1 处修改未提交")).toBeInTheDocument();
    expect(useTabs.getState().pending["redis:mock-redis:0"]).toBe(1);

    await user.keyboard("{Meta>}s{/Meta}");
    const commands = await screen.findByRole("list", { name: "将要执行的命令" });
    expect(commands).toHaveTextContent('HSET user:1001 city "苏州 西园"');
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "提交" }));

    expect(await screen.findByText("已执行 1 条命令。")).toBeInTheDocument();
    await waitFor(() => expect(valueCell(grid, "city")).not.toHaveClass("is-edited"));
    expect(valueCell(grid, "city")).toHaveTextContent("苏州 西园");
    expect(await hashValue(backend, "user:1001", "city")).toEqual({ kind: "text", value: "苏州 西园" });
    expect(useTabs.getState().pending["redis:mock-redis:0"] ?? 0).toBe(0);
  });

  it("writes nothing when someone else changed the field in between, and keeps the edit", async () => {
    const { backend, tree, user } = await renderKeys();
    await openKey(tree, user, "user", "user:1001");
    await editHashValue(user, "user:1001", "city", "苏州");
    await user.keyboard("{Meta>}s{/Meta}");
    await screen.findByRole("list", { name: "将要执行的命令" });

    // 别人在预览和提交之间把这个字段改了。
    const theirs: RowChange[] = [
      {
        kind: "update",
        key: { field: { kind: "text", value: "city" }, value: { kind: "text", value: "杭州" } },
        values: { value: "上海" },
      },
    ];
    const expected = await backend.redisPreviewKeyChanges(CONNECTION, "s:user:1001", "hash", theirs);
    await backend.redisApplyKeyChanges(CONNECTION, 0, "s:user:1001", "hash", theirs, expected);

    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "提交" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已经被别人改过或删掉了");
    expect(await hashValue(backend, "user:1001", "city")).toEqual({ kind: "text", value: "上海" });
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.getByText("1 处修改未提交")).toBeInTheDocument();
  });

  it("edits a string as text and refuses to overwrite a newer value", async () => {
    const { backend, tree, user } = await renderKeys();
    await openKey(tree, user, "counter", "counter:visits");
    expect(screen.getByLabelText("字符串的值")).toHaveTextContent("1048576");

    await user.click(screen.getByRole("button", { name: "编辑" }));
    const editor = screen.getByRole("textbox", { name: "字符串的值" });
    expect(editor).toHaveFocus();
    await user.clear(editor);
    await user.type(editor, "2000000");
    expect(useTabs.getState().pending["redis:mock-redis:0"]).toBe(1);
    await user.keyboard("{Meta>}s{/Meta}");
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "字符串的值" })).not.toBeInTheDocument());
    expect(screen.getByLabelText("字符串的值")).toHaveTextContent("2000000");

    await user.click(screen.getByRole("button", { name: "编辑" }));
    await backend.redisSetString(CONNECTION, 0, "s:counter:visits", "2000000", "2000001");
    await user.type(screen.getByRole("textbox", { name: "字符串的值" }), "9");
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已经被别人改过或删掉了");
    expect(screen.getByRole("textbox", { name: "字符串的值" })).toHaveValue("20000009");
    const detail = await backend.redisReadKey(CONNECTION, 0, "s:counter:visits");
    expect(detail.value).toMatchObject({ kind: "text", text: "2000001" });
  });

  it("sets an expiry, renames and deletes a key", async () => {
    const { tree, user } = await renderKeys();
    await openKey(tree, user, "tags", "tags:popular");
    expect(screen.getByText("不过期")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "过期时间" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "1 小时" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "设置" }));
    expect(await screen.findByText(/^还剩 (59 分|1 小时)/)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "改名" }));
    const name = screen.getByRole("textbox", { name: "新的键名" });
    await user.clear(name);
    await user.type(name, "user:1002");
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "改名" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已经有叫 user:1002 的键了");

    await user.clear(name);
    await user.type(name, "tags:hot{Enter}");
    expect(await screen.findByRole("heading", { name: "tags:hot" })).toBeInTheDocument();
    expect(await keyRow(tree, "tags:hot")).toHaveAttribute("aria-selected", "true");

    await user.click(screen.getByRole("button", { name: "删除" }));
    const confirm = screen.getByRole("dialog", { name: "删除「tags:hot」？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    expect(await screen.findByText("在左边选一个键")).toBeInTheDocument();
    await waitFor(() => expect(within(tree).queryByRole("treeitem", { name: /^tags/ })).not.toBeInTheDocument());
  });

  it("creates a key in the focused group", async () => {
    const { tree, user } = await renderKeys();
    await user.click(folder(tree, "leaderboard"));
    await user.click(screen.getByRole("button", { name: "新建键" }));
    const dialog = screen.getByRole("dialog", { name: "新建键" });
    const name = within(dialog).getByRole("textbox", { name: "键名" });
    expect(name).toHaveValue("leaderboard:");
    await user.type(name, "daily");
    await user.selectOptions(within(dialog).getByRole("combobox", { name: "类型" }), "zset");
    await user.type(within(dialog).getByRole("textbox", { name: "第一个成员" }), "黄勇");
    const score = within(dialog).getByRole("textbox", { name: "分数" });
    await user.clear(score);
    await user.type(score, "12.5");
    await user.click(within(dialog).getByRole("button", { name: "新建" }));

    expect(await screen.findByRole("heading", { name: "leaderboard:daily" })).toBeInTheDocument();
    const grid = await screen.findByRole("grid", { name: "leaderboard:daily 的值" });
    expect(within(grid).getByText("黄勇")).toBeInTheDocument();
    expect(await keyRow(tree, "leaderboard:daily")).toHaveAttribute("aria-selected", "true");

    await user.click(screen.getByRole("button", { name: "新建键" }));
    const again = screen.getByRole("dialog", { name: "新建键" });
    await user.clear(within(again).getByRole("textbox", { name: "键名" }));
    await user.type(within(again).getByRole("textbox", { name: "键名" }), "leaderboard:daily");
    await user.type(within(again).getByRole("textbox", { name: "值" }), "1");
    await user.click(within(again).getByRole("button", { name: "新建" }));
    expect(await within(again).findByRole("alert")).toHaveTextContent("已经有同名的键了");
  });

  it("asks before switching away from uncommitted edits", async () => {
    const { tree, user } = await renderKeys();
    await openKey(tree, user, "user", "user:1001");
    await editHashValue(user, "user:1001", "vip", "1");

    await user.click(await keyRow(tree, "user:1002"));
    const prompt = await screen.findByRole("dialog", { name: "放弃未提交的修改？" });
    expect(prompt).toHaveTextContent("换一个键后就没有了");
    await user.click(within(prompt).getByRole("button", { name: "取消" }));
    expect(screen.getByRole("heading", { name: "user:1001" })).toBeInTheDocument();
    expect(screen.getByText("1 处修改未提交")).toBeInTheDocument();

    await user.click(await keyRow(tree, "user:1002"));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "放弃修改并换一个键" }));
    expect(await screen.findByRole("heading", { name: "user:1002" })).toBeInTheDocument();
    await waitFor(() => expect(useTabs.getState().pending["redis:mock-redis:0"] ?? 0).toBe(0));
  });

  it("checks a string edit against the value it started from, not a newer read", async () => {
    const { backend, tree, user } = await renderKeys();
    await openKey(tree, user, "counter", "counter:visits");
    await user.click(screen.getByRole("button", { name: "编辑" }));
    await user.type(screen.getByRole("textbox", { name: "字符串的值" }), "0");

    // 别人改了值,之后这边因为设过期时间重读了一次:保存时还得按开始编辑时的值核对。
    await backend.redisSetString(CONNECTION, 0, "s:counter:visits", "1048576", "999");
    await user.click(screen.getByRole("button", { name: "过期时间" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "1 小时" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "设置" }));
    expect(await screen.findByText(/开始编辑之后服务器上的值变了/)).toBeInTheDocument();

    await user.keyboard("{Meta>}s{/Meta}");
    expect(await screen.findByRole("alert")).toHaveTextContent("已经被别人改过或删掉了");
    const detail = await backend.redisReadKey(CONNECTION, 0, "s:counter:visits");
    expect(detail.value).toMatchObject({ kind: "text", text: "999" });
  });

  it("won't edit a string with carriage returns in a text box", async () => {
    const { backend, tree, user } = await renderKeys();
    await backend.redisCreateKey(CONNECTION, 0, {
      name: "a:crlf",
      kind: "string",
      ttlSeconds: null,
      field: null,
      value: "line1\r\nline2",
      score: null,
    });
    await user.click(screen.getByRole("button", { name: "刷新键列表" }));
    await waitFor(() => expect(folder(tree, "a")).toBeInTheDocument());
    await openKey(tree, user, "a", "a:crlf");
    expect(screen.getByRole("button", { name: "编辑" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "编辑" })).toHaveAttribute("title", expect.stringContaining("回车符"));
  });

  it("asks before renaming, deleting or creating over uncommitted edits, and clears the count", async () => {
    const { tree, user, tabId } = await renderKeys();
    await openKey(tree, user, "user", "user:1001");
    await editHashValue(user, "user:1001", "vip", "1");

    await user.click(screen.getByRole("button", { name: "改名" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("textbox", { name: "新的键名" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "新建键" }));
    const dialog = screen.getByRole("dialog", { name: "新建键" });
    await user.clear(within(dialog).getByRole("textbox", { name: "键名" }));
    await user.type(within(dialog).getByRole("textbox", { name: "键名" }), "fresh");
    await user.click(within(dialog).getByRole("button", { name: "新建" }));
    const prompt = await screen.findByRole("dialog", { name: "放弃未提交的修改？" });
    await user.click(within(prompt).getByRole("button", { name: "取消" }));
    expect(screen.getByRole("heading", { name: "user:1001" })).toBeInTheDocument();
    expect(screen.getByText("1 处修改未提交")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "删除" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "放弃修改并删除" }));
    const confirm = await screen.findByRole("dialog", { name: "删除「user:1001」？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    expect(await screen.findByText("在左边选一个键")).toBeInTheDocument();
    expect(useTabs.getState().pending[tabId] ?? 0).toBe(0);
  });

  it("forgets a deleted key's value when a key with the same name comes back", async () => {
    const { backend, tree, user } = await renderKeys();
    await openKey(tree, user, "tags", "tags:popular");
    await user.click(screen.getByRole("button", { name: "删除" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除" }));
    await screen.findByText("在左边选一个键");

    await backend.redisCreateKey(CONNECTION, 0, {
      name: "tags:popular",
      kind: "string",
      ttlSeconds: null,
      field: null,
      value: "又建了一次",
      score: null,
    });
    await user.click(screen.getByRole("button", { name: "刷新键列表" }));
    // tags 分组删之前展开过,还记着展开。
    await user.click(await keyRow(tree, "tags:popular"));
    expect(await screen.findByText("又建了一次")).toBeInTheDocument();
  });

  it("keeps the keyboard cursor on the same row when a group above opens", async () => {
    const { tree, user } = await renderKeys();
    tree.focus();
    await user.keyboard("{ArrowDown}");
    const counter = folder(tree, "counter");
    expect(tree).toHaveAttribute("aria-activedescendant", counter.id);
    await user.click(folder(tree, "config"));
    tree.focus();
    await user.keyboard("{ArrowDown}");
    // 游标在 config 上(点过它),往下一行是 config 里的键,而不是错位到别处。
    expect(await screen.findByRole("heading", { name: "config:feature_flags" })).toBeInTheDocument();
  });

  it("says when the selected key is gone", async () => {
    const { backend, tree, user } = await renderKeys();
    await openKey(tree, user, ["rate", "ip"], "rate:ip:10.0.8.31");
    await backend.redisDeleteKeys(CONNECTION, 0, ["s:rate:ip:10.0.8.31"]);
    await user.click(screen.getByRole("button", { name: "重新读取这个键" }));
    expect(await screen.findByText("这个键已经不存在了")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "刷新列表" }));
    expect(await screen.findByText("在左边选一个键")).toBeInTheDocument();
    await waitFor(() => expect(within(tree).queryByRole("treeitem", { name: /^rate/ })).not.toBeInTheDocument());
  });
});
