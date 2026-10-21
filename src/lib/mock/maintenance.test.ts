// mock 的表维护:各家的语句、只有 VACUUM / REINDEX 量大小、没有的维护被拒绝。
import { describe, expect, it } from "vitest";
import { mockMaintenance, mockReferences } from "./maintenance";

const table = { database: "folio", schema: "public", table: "users" };

describe("mockMaintenance", () => {
  it("writes each database's statement", () => {
    expect(mockMaintenance("postgres", table, "reindex")).toMatchObject({
      sql: 'REINDEX TABLE "public"."users"',
      messages: [],
    });
    expect(mockMaintenance("postgres", table, "analyze")).toMatchObject({
      sql: 'ANALYZE "public"."users"',
      size: null,
    });
    expect(mockMaintenance("mariadb", table, "repair").messages[0]!.level).toBe("note");
    expect(mockMaintenance("mysql", table, "check").sql).toBe("CHECK TABLE `folio`.`users`");
  });

  it("refuses what a database cannot do", () => {
    expect(() => mockMaintenance("sqlite", table, "repair")).toThrow(
      expect.objectContaining({ code: "unsupported" }) as Error,
    );
  });

  it("knows which sample tables reference which", () => {
    const shop = { database: "shop", schema: null };
    expect(mockReferences({ ...shop, table: "customers" })).toEqual([
      { table: { ...shop, table: "orders" }, name: "fk_orders_customer", onDelete: "cascade" },
    ]);
    expect(mockReferences({ ...shop, table: "products" })[0]!.onDelete).toBe("restrict");
    expect(mockReferences({ ...shop, table: "orders" })).toEqual([]);
  });
});
