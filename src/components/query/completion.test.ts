import { describe, expect, it } from "vitest";
import { completionTarget, referencedTables, resolveQualifier } from "./completion";

describe("completionTarget", () => {
  it("reads the word being typed", () => {
    expect(completionTarget("SELECT na")).toEqual({ qualifier: null, word: "na" });
    expect(completionTarget("SELECT ")).toEqual({ qualifier: null, word: "" });
    expect(completionTarget("SELECT 订")).toEqual({ qualifier: null, word: "订" });
  });

  it("reads bare and quoted qualifiers", () => {
    expect(completionTarget("SELECT o.")).toEqual({ qualifier: "o", word: "" });
    expect(completionTarget("SELECT o.cu")).toEqual({ qualifier: "o", word: "cu" });
    expect(completionTarget("SELECT `my``db`.ta")).toEqual({ qualifier: "my`db", word: "ta" });
    expect(completionTarget('SELECT "Order Items".i')).toEqual({ qualifier: "Order Items", word: "i" });
    expect(completionTarget("SELECT 1.")).toEqual({ qualifier: "1", word: "" });
    expect(completionTarget(".x")).toEqual({ qualifier: null, word: "x" });
  });
});

describe("referencedTables", () => {
  it("finds tables after FROM and JOIN with their aliases", () => {
    const sql = "SELECT * FROM orders o JOIN customers AS c ON c.id = o.customer_id LEFT JOIN shop.items WHERE 1";
    expect(referencedTables(sql)).toEqual([
      { namespace: null, table: "orders", alias: "o" },
      { namespace: null, table: "customers", alias: "c" },
      { namespace: "shop", table: "items", alias: null },
    ]);
  });

  it("handles comma lists, quoted names and keywords that are not aliases", () => {
    const sql = 'SELECT * FROM `order items` oi, "public"."users" u, tags WHERE x';
    expect(referencedTables(sql)).toEqual([
      { namespace: null, table: "order items", alias: "oi" },
      { namespace: "public", table: "users", alias: "u" },
      { namespace: null, table: "tags", alias: null },
    ]);
    expect(referencedTables("UPDATE accounts SET a = 1")).toEqual([
      { namespace: null, table: "accounts", alias: null },
    ]);
    expect(referencedTables("INSERT INTO log VALUES (1)")).toEqual([{ namespace: null, table: "log", alias: null }]);
  });

  it("ignores FROM inside strings and comments, and dedupes", () => {
    const sql = "SELECT 'from fake' -- from other\nFROM real r JOIN real r";
    expect(referencedTables(sql)).toEqual([{ namespace: null, table: "real", alias: "r" }]);
  });
});

describe("resolveQualifier", () => {
  const refs = referencedTables("SELECT * FROM orders o JOIN Customers ON 1");
  it("prefers aliases, then table names, case-insensitively", () => {
    expect(resolveQualifier("O", refs)?.table).toBe("orders");
    expect(resolveQualifier("customers", refs)?.table).toBe("Customers");
    expect(resolveQualifier("shop", refs)).toBeNull();
  });
});
