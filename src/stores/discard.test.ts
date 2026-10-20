import { beforeEach, describe, expect, it, vi } from "vitest";
import { guardDiscard } from "./discard";
import { objectsTab, tableDataTab, useTabs } from "./tabs";
import { useUi } from "./ui";

const orders = tableDataTab("c1", "shop", null, "orders");
const users = tableDataTab("c2", "app", "public", "users");
const shop = objectsTab("c1", "shop", null);

beforeEach(() => {
  useTabs.setState({ tabs: [shop, orders, users], activeId: orders.id, pending: {} });
  useUi.setState({ discardPrompt: null });
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

  it("forgets a tab once its pending count drops to zero", () => {
    const { setPending } = useTabs.getState();
    setPending(orders.id, 1);
    setPending(orders.id, 0);
    expect(useTabs.getState().pending).toEqual({});
  });
});
