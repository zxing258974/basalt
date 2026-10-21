import { beforeEach, describe, expect, it, vi } from "vitest";
import { guardDiscard, guardQuit, hasUnsavedWork } from "./discard";
import { discardText } from "../components/data/DiscardChangesDialog";
import { objectsTab, queryTab, tableDataTab, tableDesignTab, useTabs } from "./tabs";
import { useUi } from "./ui";

const orders = tableDataTab("c1", "shop", null, "orders");
const users = tableDataTab("c2", "app", "public", "users");
const shop = objectsTab("c1", "shop", null);
const report = queryTab("c1", { database: "shop", schema: null }, "查询 1");

beforeEach(() => {
  useTabs.setState({ tabs: [shop, orders, users, report], activeId: orders.id, pending: {}, unsaved: {} });
  useUi.setState({ discardPrompt: null });
});

describe("unsaved queries and table designs together", () => {
  const design = tableDesignTab("c1", "shop", null, "orders");

  it("counts a table designer as a design, never as an unsaved query", () => {
    useTabs.setState((s) => ({ tabs: [...s.tabs, design] }));
    useTabs.getState().setUnsaved(report.id, true);
    useTabs.getState().setUnsaved(design.id, true);
    guardDiscard((t) => t.connectionId === "c1", vi.fn());
    expect(useUi.getState().discardPrompt).toMatchObject({ count: 0, queries: 1, designs: 1 });
  });

  it("names every kind of loss in the prompt", () => {
    expect(discardText(1, 1, 1)).toEqual({
      title: "放弃未保存的修改？",
      message: "有 1 行修改还没有提交、1 个查询还没有保存、1 张表的结构修改还没有保存，关闭后就没有了。",
    });
  });
});

describe("guardDiscard", () => {
  it("runs the action right away when nothing is pending", () => {
    const action = vi.fn();
    guardDiscard((t) => t.id === orders.id, action);
    expect(action).toHaveBeenCalledOnce();
    expect(useUi.getState().discardPrompt).toBeNull();
  });

  it("asks first and runs the action only after confirming", () => {
    useTabs.getState().setPending(orders.id, 2);
    const action = vi.fn();
    const cancel = vi.fn();
    guardDiscard((t) => t.connectionId === "c1", action, cancel);

    const prompt = useUi.getState().discardPrompt;
    expect(prompt?.count).toBe(2);
    expect(action).not.toHaveBeenCalled();
    prompt!.onConfirm();
    expect(action).toHaveBeenCalledOnce();
  });

  it("only counts the tabs being closed", () => {
    useTabs.getState().setPending(users.id, 3);
    const action = vi.fn();
    guardDiscard((t) => t.id === orders.id, action);
    expect(action).toHaveBeenCalledOnce();
  });

  it("counts unsaved query tabs separately", () => {
    useTabs.getState().setPending(orders.id, 1);
    useTabs.getState().setUnsaved(report.id, true);
    const action = vi.fn();
    guardDiscard((t) => t.connectionId === "c1", action);
    expect(useUi.getState().discardPrompt).toMatchObject({ count: 1, queries: 1 });
    expect(action).not.toHaveBeenCalled();
  });

  it("words the prompt by what would be lost", () => {
    expect(discardText(2, 0, 0)).toEqual({
      title: "放弃未提交的修改？",
      message: "有 2 行修改还没有提交，关闭后就没有了。",
    });
    expect(discardText(0, 1, 0)).toEqual({
      title: "放弃未保存的查询？",
      message: "有 1 个查询还没有保存，关闭后就没有了。",
    });
    expect(discardText(3, 2, 0).message).toBe("有 3 行修改还没有提交、2 个查询还没有保存，关闭后就没有了。");
    expect(discardText(3, 2, 0).title).toBe("放弃未保存的修改？");
    expect(discardText(2, 0, 0, false, "清空").message).toBe("有 2 行修改还没有提交，清空后就没有了。");
    expect(discardText(2, 0, 0, true, "清空").message).toBe("有 2 行修改还没有提交，退出后就没有了。");
  });

  it("cancels a prompt that is still open before asking again", () => {
    useTabs.getState().setPending(orders.id, 1);
    const firstCancel = vi.fn();
    guardDiscard((t) => t.id === orders.id, vi.fn(), firstCancel);
    guardDiscard((t) => t.connectionId === "c1", vi.fn());
    expect(firstCancel).toHaveBeenCalledOnce();
  });

  it("forgets a tab once its pending count drops to zero", () => {
    const { setPending } = useTabs.getState();
    setPending(orders.id, 1);
    setPending(orders.id, 0);
    expect(useTabs.getState().pending).toEqual({});
  });
});

describe("hasUnsavedWork", () => {
  it("is false when no open tab has pending rows or unsaved SQL", () => {
    expect(hasUnsavedWork(useTabs.getState())).toBe(false);
  });

  it("sees table edits, unsaved queries and unsaved table designs on any open tab", () => {
    useTabs.getState().setPending(users.id, 1);
    expect(hasUnsavedWork(useTabs.getState())).toBe(true);
    useTabs.getState().setPending(users.id, 0);
    useTabs.getState().setUnsaved(report.id, true);
    expect(hasUnsavedWork(useTabs.getState())).toBe(true);
    useTabs.getState().setUnsaved(report.id, false);
    const design = tableDesignTab("c2", "app", "public", "users");
    useTabs.setState((s) => ({ tabs: [...s.tabs, design] }));
    useTabs.getState().setUnsaved(design.id, true);
    expect(hasUnsavedWork(useTabs.getState())).toBe(true);
  });

  it("ignores leftovers of tabs that are no longer open", () => {
    useTabs.setState({ tabs: [shop], pending: { [orders.id]: 2 }, unsaved: { [report.id]: true } });
    expect(hasUnsavedWork(useTabs.getState())).toBe(false);
  });
});

describe("guardQuit", () => {
  it("quits right away when nothing would be lost", () => {
    const quit = vi.fn();
    guardQuit(quit);
    expect(quit).toHaveBeenCalledOnce();
    expect(useUi.getState().discardPrompt).toBeNull();
  });

  it("counts every open tab and marks the prompt as quitting", () => {
    const design = tableDesignTab("c2", "app", "public", "users");
    useTabs.setState((s) => ({ tabs: [...s.tabs, design] }));
    useTabs.getState().setPending(orders.id, 2);
    useTabs.getState().setPending(users.id, 1);
    useTabs.getState().setUnsaved(report.id, true);
    useTabs.getState().setUnsaved(design.id, true);
    const quit = vi.fn();
    guardQuit(quit);

    const prompt = useUi.getState().discardPrompt;
    expect(prompt).toMatchObject({ count: 3, queries: 1, designs: 1, quit: true });
    expect(quit).not.toHaveBeenCalled();
    prompt!.onConfirm();
    expect(quit).toHaveBeenCalledOnce();
  });

  it("says quitting instead of closing in the prompt", () => {
    expect(discardText(1, 2, 0, true).message).toBe("有 1 行修改还没有提交、2 个查询还没有保存，退出后就没有了。");
    expect(discardText(0, 0, 1, true).message).toBe("有 1 张表的结构修改还没有保存，退出后就没有了。");
  });
});
