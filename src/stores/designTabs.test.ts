// 设计器标签页:id 与标题、建表改名后原地换成新标签、没保存的表结构关闭前确认。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { discardText } from "../components/data/DiscardChangesDialog";
import { guardDiscard } from "./discard";
import { newTableTab, objectsTab, replaceTab, tableDataTab, tableDesignTab, useTabs } from "./tabs";
import { useUi } from "./ui";

const shop = objectsTab("c1", "shop", null);
const design = tableDesignTab("c1", "shop", null, "orders");
const data = tableDataTab("c1", "shop", null, "orders");

beforeEach(() => {
  useTabs.setState({ tabs: [shop, design, data], activeId: design.id, pending: {}, unsaved: {} });
  useUi.setState({ discardPrompt: null });
});

describe("design tabs", () => {
  it("are keyed by table and never collide with the data tab", () => {
    expect(design).toMatchObject({ kind: "tableDesign", title: "orders", table: "orders", schema: null });
    expect(design.id).not.toBe(data.id);
    expect(tableDesignTab("c1", "a:b", "s", "t").id).toBe("design:c1:a%3Ab:s:t");
    const fresh = newTableTab("c1", "shop", null);
    expect(fresh).toMatchObject({ title: "新表", table: null });
    expect(newTableTab("c1", "shop", null).id).not.toBe(fresh.id);
  });

  it("replace in place, or close the old one when the new one is already open", () => {
    const renamed = tableDesignTab("c1", "shop", null, "purchases");
    const next = replaceTab(useTabs.getState(), design.id, renamed);
    expect(next.tabs.map((t) => t.id)).toEqual([shop.id, renamed.id, data.id]);
    expect(next.activeId).toBe(renamed.id);

    const background = replaceTab({ tabs: [shop, design], activeId: shop.id }, design.id, renamed);
    expect(background.activeId).toBe(shop.id);

    const existing = replaceTab({ tabs: [shop, design, renamed], activeId: design.id }, design.id, renamed);
    expect(existing.tabs.map((t) => t.id)).toEqual([shop.id, renamed.id]);
    expect(existing.activeId).toBe(renamed.id);

    const same = useTabs.getState();
    expect(replaceTab(same, design.id, design)).toBe(same);
    useTabs.getState().replace(design.id, renamed);
    expect(useTabs.getState().activeId).toBe(renamed.id);
  });

  it("track unsaved designs without churning the store", () => {
    const { setUnsaved } = useTabs.getState();
    setUnsaved(design.id, true);
    const marked = useTabs.getState().unsaved;
    setUnsaved(design.id, true);
    expect(useTabs.getState().unsaved).toBe(marked);
    setUnsaved(design.id, false);
    expect(useTabs.getState().unsaved).toEqual({});
  });
});

describe("closing with unsaved designs", () => {
  it("asks first and counts designs separately from row edits", () => {
    useTabs.getState().setUnsaved(design.id, true);
    useTabs.getState().setPending(data.id, 2);
    const action = vi.fn();
    guardDiscard((t) => t.connectionId === "c1", action);
    expect(useUi.getState().discardPrompt).toMatchObject({ count: 2, designs: 1 });
    expect(action).not.toHaveBeenCalled();
  });

  it("ignores unsaved flags on tabs that are not designers", () => {
    useTabs.getState().setUnsaved(data.id, true);
    const action = vi.fn();
    guardDiscard((t) => t.id === data.id, action);
    expect(action).toHaveBeenCalledOnce();
  });

  it("words the prompt for rows, designs or both", () => {
    expect(discardText(3, 0, 0)).toEqual({
      title: "放弃未提交的修改？",
      message: "有 3 行修改还没有提交，关闭后就没有了。",
    });
    expect(discardText(0, 0, 1)).toEqual({
      title: "放弃未保存的修改？",
      message: "有 1 张表的结构修改还没有保存，关闭后就没有了。",
    });
    expect(discardText(2, 0, 2).message).toBe("有 2 行修改还没有提交、2 张表的结构修改还没有保存，关闭后就没有了。");
  });
});
