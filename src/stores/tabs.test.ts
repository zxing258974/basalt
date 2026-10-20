import { describe, expect, it } from "vitest";
import {
  closeConnectionTabs,
  closeTab,
  nextQueryTitle,
  objectsTab,
  openTab,
  queryTab,
  sameSavedQuery,
  tableDataTab,
  type TabsState,
  updateQueryTab,
  useTabs,
} from "./tabs";

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

describe("tableDataTab", () => {
  it("names the tab after the table and keeps names with separators apart", () => {
    const orders = tableDataTab("c1", "shop", null, "orders");
    expect(orders.kind).toBe("tableData");
    expect(orders.title).toBe("orders");
    expect(orders.table).toEqual({ database: "shop", schema: null, table: "orders" });
    expect(tableDataTab("c1", "a:b", null, "c").id).not.toBe(tableDataTab("c1", "a", "b", "c").id);
    expect(orders.id).not.toBe(objectsTab("c1", "shop", null).id);
  });

  it("closes together with its connection", () => {
    const state = [shop, tableDataTab("c1", "shop", null, "orders"), pg].reduce(openTab, empty);
    expect(closeConnectionTabs(state, "c1").tabs).toEqual([pg]);
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

describe("query tabs", () => {
  const scope = { database: "shop", schema: null };
  const saved = { connectionId: "c1", database: "shop", schema: null, name: "日报" };

  it("numbers untitled queries after the highest one still open", () => {
    expect(nextQueryTitle([])).toBe("查询 1");
    const tabs = [
      queryTab("c1", scope, "查询 1"),
      queryTab("c1", scope, "查询 4"),
      queryTab("c1", scope, "查询 9", saved),
    ];
    expect(nextQueryTitle(tabs)).toBe("查询 5");
  });

  it("gives every query tab its own id", () => {
    expect(queryTab("c1", scope, "a").id).not.toBe(queryTab("c1", scope, "a").id);
  });

  it("updates only the query tab it is asked to", () => {
    const query = queryTab("c1", scope, "查询 1");
    const objects = objectsTab("c1", "shop", null);
    const state = { tabs: [objects, query], activeId: query.id };
    const next = updateQueryTab(state, query.id, { title: "日报", saved });
    expect(next.tabs[1]).toMatchObject({ id: query.id, title: "日报", saved });
    expect(next.tabs[0]).toBe(objects);
    expect(updateQueryTab(state, objects.id, { title: "x" }).tabs[0]).toBe(objects);
  });

  it("compares saved query locations field by field", () => {
    expect(sameSavedQuery(saved, { ...saved })).toBe(true);
    expect(sameSavedQuery(saved, { ...saved, schema: "public" })).toBe(false);
    expect(sameSavedQuery(null, saved)).toBe(false);
  });

  it("tracks unsaved query tabs", () => {
    const { setUnsaved } = useTabs.getState();
    setUnsaved("q1", true);
    setUnsaved("q1", true);
    expect(useTabs.getState().unsaved).toEqual({ q1: true });
    setUnsaved("q1", false);
    expect(useTabs.getState().unsaved).toEqual({});
  });
});

describe("closing a connection's tabs", () => {
  it("can keep query tabs so unsaved SQL survives a reconnect", () => {
    const query = queryTab("c1", { database: "shop", schema: null }, "查询 1");
    const state = { tabs: [shop, query, pg], activeId: query.id };
    expect(closeConnectionTabs(state, "c1", true).tabs).toEqual([query, pg]);
    expect(closeConnectionTabs(state, "c1").tabs).toEqual([pg]);
  });
});
