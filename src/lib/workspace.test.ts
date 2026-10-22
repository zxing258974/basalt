// 标签页记录的格式:记什么、读回来时丢掉什么、等连接的标签页怎么按原位置插回去。
import { describe, expect, it } from "vitest";
import {
  monitorTab,
  newTableTab,
  objectsTab,
  queryTab,
  tableDataTab,
  tableDesignTab,
  type QueryTab,
} from "../stores/tabs";
import {
  insertByOrder,
  parseWorkspace,
  planRestore,
  restoredTab,
  snapshot,
  WORKSPACE_VERSION,
  type SavedQueryTab,
} from "./workspace";

const scope = { database: "shop", schema: null };
const draft = { ...queryTab("c1", scope, "查询 1"), id: "query:1" };
const report: QueryTab = {
  ...queryTab("c1", scope, "日报", { connectionId: "c1", database: "shop", schema: null, name: "日报" }, "SELECT 1;"),
  id: "query:2",
};
const orders = tableDataTab("c1", "shop", null, "orders");
const shop = objectsTab("c2", "app", "public");
const design = tableDesignTab("c1", "shop", null, "orders");
const monitor = monitorTab("c2");

describe("snapshot", () => {
  it("records the SQL being edited and what it was when last saved", () => {
    const saved = snapshot(
      { tabs: [draft, orders, report], activeId: orders.id },
      { "query:1": { sql: "SELECT 42;", baseline: "" } },
    );
    expect(saved.version).toBe(WORKSPACE_VERSION);
    expect(saved.activeId).toBe(orders.id);
    expect(saved.tabs.map((t) => t.id)).toEqual(["query:1", orders.id, "query:2"]);
    expect(saved.tabs[0]).toMatchObject({ kind: "query", sql: "SELECT 42;", baselineSql: "" });
    expect(saved.tabs[0]).not.toHaveProperty("initialSql");
    // 还没打开过编辑器的(刚恢复回来):照标签页上记的。
    expect(saved.tabs[2]).toMatchObject({ sql: "SELECT 1;", baselineSql: "SELECT 1;" });
    const restored = { ...report, initialSql: "SELECT 2;", baselineSql: "SELECT 1;" };
    expect(snapshot({ tabs: [restored], activeId: null }, {}).tabs[0]).toMatchObject({
      sql: "SELECT 2;",
      baselineSql: "SELECT 1;",
    });
  });

  it("leaves out designs of tables that do not exist yet", () => {
    const fresh = newTableTab("c1", "shop", null);
    const saved = snapshot({ tabs: [fresh, design], activeId: fresh.id }, {});
    expect(saved.tabs.map((t) => t.id)).toEqual([design.id]);
    expect(saved.activeId).toBeNull();
  });

  it("keeps tabs still waiting for their connection where they were", () => {
    const order = ["query:1", orders.id, shop.id, "query:2"];
    const saved = snapshot({ tabs: [draft, report], activeId: draft.id }, {}, [shop, orders], order);
    expect(saved.tabs.map((t) => t.id)).toEqual(order);
  });
});

describe("insertByOrder", () => {
  const order = ["a", "b", "c", "d"];
  const ids = (list: { id: string }[]) => list.map((t) => t.id);

  it("goes right after the closest tab that came before it last time", () => {
    expect(ids(insertByOrder([{ id: "a" }, { id: "d" }], { id: "c" }, order))).toEqual(["a", "c", "d"]);
    expect(ids(insertByOrder([{ id: "d" }], { id: "a" }, order))).toEqual(["a", "d"]);
    expect(ids(insertByOrder([{ id: "x" }, { id: "b" }], { id: "c" }, order))).toEqual(["x", "b", "c"]);
  });

  it("goes to the end when it was not there last time", () => {
    expect(ids(insertByOrder([{ id: "a" }], { id: "new" }, order))).toEqual(["a", "new"]);
  });
});

describe("parseWorkspace", () => {
  const good = snapshot({ tabs: [draft, orders, shop, design, monitor], activeId: shop.id }, {});

  it("reads back what was written", () => {
    expect(parseWorkspace(JSON.stringify(good))).toEqual(good);
  });

  it("gives up on files it does not understand", () => {
    expect(parseWorkspace("{写到一半")).toBeNull();
    expect(parseWorkspace(JSON.stringify({ ...good, version: 99 }))).toBeNull();
    expect(parseWorkspace(JSON.stringify({ version: 1, tabs: "x" }))).toBeNull();
    expect(parseWorkspace("[]")).toBeNull();
  });

  it("drops broken or repeated tabs but keeps the rest", () => {
    const broken = [
      { kind: "query", id: "q:x", title: "坏的", connectionId: "c1", scope, saved: null },
      { kind: "tableData", id: "t:x", title: "坏的", connectionId: "c1", table: { database: "shop" } },
      { kind: "unknown", id: "u", title: "?", connectionId: "c1" },
      { kind: "monitor", id: "monitor:c9", title: "服务器监控", connectionId: "c1" },
      "not a tab",
      good.tabs[0],
    ];
    const parsed = parseWorkspace(JSON.stringify({ ...good, tabs: [...good.tabs, ...broken], activeId: "q:x" }));
    expect(parsed?.tabs.map((t) => t.id)).toEqual(good.tabs.map((t) => t.id));
    expect(parsed?.activeId).toBeNull();
  });
});

describe("planRestore", () => {
  const saved = snapshot({ tabs: [draft, orders, shop, report, monitor], activeId: orders.id }, {});

  it("brings queries back right away and waits for the connection for the rest", () => {
    const plan = planRestore(saved, new Set(["c1", "c2"]));
    expect(plan.now.map((t) => t.id)).toEqual(["query:1", "query:2"]);
    expect([...plan.waiting.keys()]).toEqual(["c1", "c2"]);
    expect(plan.waiting.get("c1")?.map((t) => t.id)).toEqual([orders.id]);
    expect(plan.waiting.get("c2")?.map((t) => t.id)).toEqual([shop.id, "monitor:c2"]);
    expect(plan.order).toEqual(saved.tabs.map((t) => t.id));
    expect(plan.activeId).toBe(orders.id);
  });

  it("keeps queries of a connection that is gone, but not its other tabs", () => {
    const plan = planRestore(saved, new Set(["c2"]));
    expect(plan.now.map((t) => t.id)).toEqual(["query:1", "query:2"]);
    expect([...plan.waiting.keys()]).toEqual(["c2"]);
  });

  it("turns a recorded query back into an editor tab", () => {
    const recorded = saved.tabs[3] as SavedQueryTab;
    expect(restoredTab({ ...recorded, sql: "SELECT 2;" })).toMatchObject({
      kind: "query",
      initialSql: "SELECT 2;",
      baselineSql: "SELECT 1;",
      saved: report.saved,
    });
  });
});
