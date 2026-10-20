import { describe, expect, it } from "vitest";
import { closeConnectionTabs, closeTab, objectsTab, openTab, type TabsState } from "./tabs";

const empty: TabsState = { tabs: [], activeId: null };
const shop = objectsTab("c1", "shop", null);
const crm = objectsTab("c1", "crm", null);
const pg = objectsTab("c2", "folio", "public");

describe("objectsTab", () => {
  it("builds a stable id and a readable title", () => {
    expect(shop.id).toBe("objects:c1:shop:");
    expect(shop.title).toBe("shop");
    expect(pg.id).toBe("objects:c2:folio:public");
    expect(pg.title).toBe("folio.public");
  });
});

describe("openTab", () => {
  it("appends a new tab and activates it without mutating the input", () => {
    const next = openTab(empty, shop);
    expect(next.tabs).toEqual([shop]);
    expect(next.activeId).toBe(shop.id);
    expect(empty.tabs).toEqual([]);
  });

  it("re-activates an existing tab instead of duplicating it", () => {
    const state = openTab(openTab(empty, shop), crm);
    const next = openTab(state, objectsTab("c1", "shop", null));
    expect(next.tabs.map((t) => t.id)).toEqual([shop.id, crm.id]);
    expect(next.activeId).toBe(shop.id);
  });
});

describe("closeTab", () => {
  const three = [shop, crm, pg].reduce(openTab, empty);

  it("activates the right neighbour when closing the active tab", () => {
    const state = { ...three, activeId: crm.id };
    const next = closeTab(state, crm.id);
    expect(next.tabs.map((t) => t.id)).toEqual([shop.id, pg.id]);
    expect(next.activeId).toBe(pg.id);
  });

  it("falls back to the left neighbour when closing the last tab", () => {
    const next = closeTab(three, pg.id);
    expect(next.activeId).toBe(crm.id);
  });

  it("keeps the active tab when closing a background tab", () => {
    const next = closeTab(three, shop.id);
    expect(next.activeId).toBe(pg.id);
  });

  it("clears the active id when the only tab closes", () => {
    expect(closeTab(openTab(empty, shop), shop.id)).toEqual(empty);
  });

  it("ignores unknown ids", () => {
    expect(closeTab(three, "nope")).toBe(three);
  });
});

describe("closeConnectionTabs", () => {
  it("drops every tab of the connection and re-targets the active tab", () => {
    const state = { ...[shop, pg, crm].reduce(openTab, empty), activeId: crm.id };
    const next = closeConnectionTabs(state, "c1");
    expect(next.tabs.map((t) => t.id)).toEqual([pg.id]);
    expect(next.activeId).toBe(pg.id);
  });

  it("returns the same state when nothing belongs to the connection", () => {
    const state = openTab(empty, pg);
    expect(closeConnectionTabs(state, "c1")).toBe(state);
  });
});
