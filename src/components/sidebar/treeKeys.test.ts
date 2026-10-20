import { describe, expect, it } from "vitest";
import { connectionKey, databaseKey, groupKey, schemaKey, tableKey } from "./treeKeys";

describe("tree keys", () => {
  it("nests every child key under its parent so prefix matching collapses a subtree", () => {
    const c = connectionKey("c1");
    const d = databaseKey("c1", "shop");
    const s = schemaKey("c1", "shop", "public");
    const g = groupKey("c1", "shop", "public", "tables");
    const t = tableKey("c1", "shop", "public", "orders");
    for (const child of [d, s, g, t]) expect(child.startsWith(`${c}/`)).toBe(true);
    expect(s.startsWith(`${d}/`)).toBe(true);
    expect(g.startsWith(`${s}/`)).toBe(true);
  });

  it("does not let one connection id prefix-match another", () => {
    expect(databaseKey("c10", "x").startsWith(`${connectionKey("c1")}/`)).toBe(false);
  });

  it("escapes separators inside names", () => {
    expect(databaseKey("c1", "a/b")).not.toBe(schemaKey("c1", "a", "b"));
  });

  it("omits the schema level when there is none", () => {
    expect(groupKey("c1", "main", null, "views")).toBe("c:c1/d:main/g:views");
  });
});

describe("connectionIdFromKey", () => {
  it("recovers the connection id from any node key", async () => {
    const { connectionIdFromKey } = await import("./treeKeys");
    expect(connectionIdFromKey(tableKey("a/b", "shop", null, "orders"))).toBe("a/b");
    expect(connectionIdFromKey(connectionKey("c1"))).toBe("c1");
    const { folderKey } = await import("./treeKeys");
    expect(connectionIdFromKey(folderKey("开发"))).toBeNull();
    expect(connectionIdFromKey(null)).toBeNull();
  });
});
