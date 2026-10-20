// mock 后端的表结构:读种子表、按方言出个大概的 DDL、校验、执行后对象列表跟着变。
import { describe, expect, it } from "vitest";
import type { TableDesign } from "../designTypes";
import type { TableInfo } from "../types";
import { createMockDesigns } from "./tableDesign";

const ref = (table: string, schema: string | null = null) => ({ database: "shop", schema, table });

function expectError(run: () => unknown, message: string) {
  expect(run).toThrow(expect.objectContaining({ code: "invalid_edit", message }) as Error);
}

describe("mock table designs", () => {
  it("reads seed tables with keys, indexes and a foreign key", () => {
    const designs = createMockDesigns();
    const orders = designs.read("c", "mysql", ref("orders"));
    expect(orders.primaryKey).toEqual(["col:id"]);
    expect(orders.foreignKeys[0]).toMatchObject({ refTable: "customers", onDelete: "restrict" });
    expect(orders.columns.find((c) => c.name === "status")!.default).toBe("'pending'");
    const customers = designs.read("c", "postgres", ref("customers", "public"));
    expect(customers.primaryKeyName).toBe("customers_pkey");
    expect(customers.indexes.map((i) => i.name)).toEqual(["uk_customers_email", "idx_customers_city"]);
    expect(designs.read("c", "mysql", ref("products")).columns.at(-1)!.generated).not.toBeNull();
    expect(designs.read("c", "sqlite", ref("notes")).options.engine).toBeNull();
  });

  it("validates the target like the backend", () => {
    const designs = createMockDesigns();
    const base = designs.read("c", "mysql", ref("customers"));
    const check = (patch: Partial<TableDesign>, message: string, driver: "mysql" | "postgres" = "mysql") =>
      expectError(() => designs.preview("c", driver, ref("customers"), { ...base, ...patch }), message);
    check({ table: ref(" ") }, "请填写表名。");
    check({ columns: [] }, "表里至少要有一个字段。");
    check({ columns: [{ ...base.columns[0]!, name: "" }] }, "第 1 个字段没有填字段名。");
    check({ columns: [base.columns[0]!, { ...base.columns[1]!, name: "ID" }] }, "字段名「ID」重复了。");
    check({ columns: [{ ...base.columns[0]!, dataType: " " }] }, "字段「id」没有填类型。");
    check({ indexes: [{ ...base.indexes[0]!, name: "" }] }, "有索引没有填索引名。");
    check({ indexes: [{ ...base.indexes[0]!, columns: [] }] }, "索引「uk_customers_email」至少选一个字段。");
    const fk = { ...designs.read("c", "mysql", ref("orders")).foreignKeys[0]! };
    check({ foreignKeys: [{ ...fk, name: "", columns: [] }] }, "第 1 个外键至少选一个字段。");
    check({ foreignKeys: [{ ...fk, refTable: "" }] }, "外键「fk_orders_customer」没有选引用的表。");
    check({ foreignKeys: [{ ...fk, refColumns: [] }] }, "外键「fk_orders_customer」的字段和引用的字段个数不一致。");
  });

  it("previews per dialect and refuses a stale preview", () => {
    const designs = createMockDesigns();
    const base = designs.read("c", "postgres", ref("customers", "public"));
    const target: TableDesign = {
      ...base,
      columns: base.columns
        .filter((c) => c.name !== "note")
        .map((c) => (c.name === "vip" ? { ...c, nullable: false } : c)),
      primaryKey: [],
      indexes: base.indexes.slice(1),
      options: { ...base.options, comment: "客户" },
    };
    const preview = designs.preview("c", "postgres", ref("customers", "public"), target);
    expect(preview.mode).toBe("transaction");
    expect(preview.statements).toEqual([
      'ALTER TABLE "public"."customers" DROP COLUMN "note"',
      'ALTER TABLE "public"."customers" MODIFY COLUMN "vip" tinyint(1) NOT NULL DEFAULT 0',
      'ALTER TABLE "public"."customers" DROP PRIMARY KEY',
      'ALTER TABLE "public"."customers" DROP INDEX "uk_customers_email"',
      `ALTER TABLE "public"."customers" COMMENT = '客户'`,
    ]);
    expectError(
      () => designs.apply("c", "postgres", ref("customers", "public"), target, ["旧的"]),
      "表结构在预览之后变了，要执行的 SQL 和预览的不一样。请重新预览后再保存。",
    );
  });

  it("creating and renaming tables shows up in the object list", () => {
    const designs = createMockDesigns();
    const base: TableInfo[] = [
      { name: "customers", kind: "table" } as TableInfo,
      { name: "orders", kind: "table" } as TableInfo,
    ];
    const created = { ...designs.read("c", "sqlite", ref("x")), table: ref("zeta") };
    const { statements } = designs.preview("c", "sqlite", null, created);
    expect(statements[0]).toMatch(/^CREATE TABLE "shop"."zeta"/);
    designs.apply("c", "sqlite", null, created, statements);
    const renamed = { ...designs.read("c", "mysql", ref("customers")), table: ref("clients") };
    const rename = designs.preview("c", "mysql", ref("customers"), renamed);
    expect(rename.statements).toEqual(["ALTER TABLE `shop`.`customers`\n  RENAME TO `clients`"]);
    designs.apply("c", "mysql", ref("customers"), renamed, rename.statements);
    expect(designs.tables("c", "shop", null, base).map((t) => t.name)).toEqual(["orders", "clients", "zeta"]);
    expect(designs.read("c", "mysql", ref("clients")).columns[0]!.id).toBe("col:id");
    expect(
      designs.preview("c", "mysql", ref("clients"), designs.read("c", "mysql", ref("clients"))).statements,
    ).toEqual([]);
  });
});
