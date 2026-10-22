import { describe, expect, it } from "vitest";
import type { ResultColumn, ResultSet } from "../../lib/queryTypes";
import type { ColumnInfo, TableRef } from "../../lib/types";
import { editPlan, sourceTable, statementShape, toTableChanges } from "./resultEditing";

const ORDERS: TableRef = { database: "shop", schema: null, table: "orders" };
const OTHER: TableRef = { database: "shop", schema: null, table: "customers" };

const col = (name: string, source: [TableRef, string] | null = null): ResultColumn => ({
  name,
  dataType: "int",
  source: source ? { table: source[0], column: source[1] } : null,
});

const set = (...columns: ResultColumn[]): ResultSet => ({
  id: 1,
  columns,
  rows: [],
  totalRows: 0,
  truncated: false,
  readOnly: null,
});

const info = (name: string, extra: Partial<ColumnInfo> = {}): ColumnInfo => ({
  name,
  dataType: "int",
  isPrimaryKey: false,
  nullable: true,
  hasDefault: false,
  isGenerated: false,
  valueType: { kind: "integer", unsigned: false },
  ...extra,
});

describe("statementShape", () => {
  it.each([
    "SELECT * FROM orders",
    "select id, total from shop.orders o where id > 1 order by id limit 10",
    "SELECT `id` FROM `orders` AS `o` FOR UPDATE",
    'SELECT "id" FROM "public"."orders"',
    "-- 说明\nSELECT id FROM orders WHERE note = 'a, b JOIN c' /* , d */",
    "SELECT id, (SELECT count(*) FROM items i WHERE i.order_id = o.id) AS n FROM orders o",
    "SELECT id FROM orders WHERE id IN (SELECT order_id FROM items);",
  ])("accepts a single table: %s", (sql) => {
    expect(statementShape(sql, "mysql").kind).toBe("single");
  });

  it.each([
    "SELECT * FROM orders o JOIN customers c ON c.id = o.customer_id",
    "SELECT * FROM orders a, orders b",
    "SELECT * FROM (SELECT * FROM orders) t",
    "SELECT id FROM orders UNION SELECT id FROM orders",
    "SELECT * FROM orders NATURAL JOIN customers",
    "SELECT 1",
    "SELECT a.id FROM orders a /*! JOIN orders b ON b.id = a.id */",
  ])("rejects anything else: %s", (sql) => {
    expect(statementShape(sql, "mysql").kind).toBe("multiple");
  });

  it("only SELECT counts", () => {
    expect(statementShape("WITH t AS (SELECT 1) SELECT * FROM t", "postgres").kind).toBe("notSelect");
    expect(statementShape("SHOW TABLES", "mysql").kind).toBe("notSelect");
    expect(statementShape("(SELECT * FROM orders)", "mysql").kind).toBe("notSelect");
  });

  it("knows each dialect's quoting", () => {
    expect(statementShape("SELECT [id] FROM [orders]", "sqlite").kind).toBe("single");
    expect(statementShape("SELECT id FROM orders # , other\nWHERE 1", "mysql").kind).toBe("single");
    expect(statementShape("SELECT id FROM orders WHERE a = 'it\\'s, x'", "mysql").kind).toBe("single");
    // PostgreSQL 的 E'…' 和 $tag$ 字符串里的东西不算语句。
    expect(statementShape("SELECT E'\\'', id FROM t", "postgres").kind).toBe("single");
    expect(statementShape("SELECT E'\\'' AS q FROM t a JOIN t b ON true", "postgres").kind).toBe("multiple");
    expect(statementShape("SELECT id FROM t WHERE x = $q$ , u JOIN v $q$", "postgres").kind).toBe("single");
    expect(statementShape("SELECT id FROM t, u WHERE x = $1", "postgres").kind).toBe("multiple");
  });

  it("returns the table name as written", () => {
    expect(statementShape('SELECT * FROM "Shop"."Orders" o', "postgres")).toEqual({
      kind: "single",
      name: [
        { text: "Shop", quoted: true },
        { text: "Orders", quoted: true },
      ],
    });
  });
});

describe("sourceTable", () => {
  const sql = "SELECT * FROM orders";

  it("finds the one table every sourced column comes from", () => {
    expect(sourceTable(sql, set(col("id", [ORDERS, "id"]), col("n")), "mysql")).toEqual({ table: ORDERS });
  });

  it("needs FROM to name the table the columns come from (a view is not its base table)", () => {
    const columns = set(col("id", [ORDERS, "id"]));
    expect(sourceTable("SELECT * FROM shop.ORDERS", columns, "mysql")).toEqual({ table: ORDERS });
    expect(sourceTable("SELECT * FROM `orders`", columns, "mysql")).toEqual({ table: ORDERS });
    const view = sourceTable("SELECT * FROM v_orders", columns, "sqlite") as { reason: string };
    expect(view.reason).toContain("不是 FROM 里写的那张");
    expect(sourceTable("SELECT * FROM other_db.orders", columns, "mysql")).toHaveProperty("reason");
    expect(sourceTable("SELECT * FROM `Orders`", columns, "mysql")).toHaveProperty("reason");
    const pg = { database: "folio", schema: "public", table: "orders" };
    const pgSet = set(col("id", [pg, "id"]));
    expect(sourceTable("SELECT * FROM public.orders", pgSet, "postgres")).toEqual({ table: pg });
    expect(sourceTable("SELECT * FROM archive.orders", pgSet, "postgres")).toHaveProperty("reason");
  });

  it("passes on the backend's reason, such as a temporary table", () => {
    const temp = { ...set(col("id", [ORDERS, "id"])), readOnly: "结果来自临时表 orders" };
    expect(sourceTable("SELECT * FROM orders", temp, "mysql")).toEqual({ reason: "结果来自临时表 orders" });
  });

  it("explains why a result can't be edited", () => {
    const reason = (s: ResultSet, text = sql) => (sourceTable(text, s, "mysql") as { reason: string }).reason;
    expect(reason(set(col("id", [ORDERS, "id"])), "SHOW TABLES")).toContain("只有 SELECT");
    expect(reason(set(col("id", [ORDERS, "id"])), "SELECT * FROM a, b")).toContain("多张表");
    expect(reason(set(col("n")))).toContain("没有直接取自表的列");
    expect(reason(set(col("id", [ORDERS, "id"]), col("name", [OTHER, "name"])))).toContain("多张表");
    expect(reason(set(col("id", [ORDERS, "id"]), col("id", [ORDERS, "code"])))).toContain("同名");
    expect(reason(set(col("id", [ORDERS, "id"]), col("again", [ORDERS, "id"])))).toContain("不止一次");
  });
});

describe("editPlan", () => {
  const table = [info("id", { isPrimaryKey: true }), info("total"), info("note")];

  it("maps result columns onto table columns and keeps aliases for display", () => {
    const plan = editPlan(set(col("id", [ORDERS, "id"]), col("amount", [ORDERS, "total"]), col("x2")), ORDERS, table);
    expect(plan.kind).toBe("editable");
    if (plan.kind !== "editable") return;
    expect(plan.columns.map((c) => [c.name, c.isPrimaryKey, c.isGenerated])).toEqual([
      ["id", true, false],
      ["amount", false, false],
      ["x2", false, true],
    ]);
    expect(plan.tableColumn).toEqual({ id: "id", amount: "total" });
  });

  it("needs a primary key, and all of it in the result", () => {
    const noKey = editPlan(set(col("id", [ORDERS, "id"])), ORDERS, [info("id")]);
    expect(noKey).toEqual({ kind: "readOnly", reason: "orders 没有主键，不能在这里改。" });
    const twoKeys = [info("a", { isPrimaryKey: true }), info("b", { isPrimaryKey: true })];
    const partial = editPlan(set(col("a", [ORDERS, "a"])), ORDERS, twoKeys);
    expect(partial).toEqual({ kind: "readOnly", reason: "结果里没有主键列 b，不能在这里改。" });
  });
});

describe("toTableChanges", () => {
  it("renames keys and values to table columns", () => {
    const names = { id: "id", amount: "total" };
    expect(
      toTableChanges(
        [
          { kind: "update", key: { id: { kind: "int", value: "1" } }, values: { amount: "9" } },
          { kind: "delete", key: { id: { kind: "int", value: "2" } } },
          { kind: "insert", values: { amount: "3" } },
        ],
        names,
      ),
    ).toEqual([
      { kind: "update", key: { id: { kind: "int", value: "1" } }, values: { total: "9" } },
      { kind: "delete", key: { id: { kind: "int", value: "2" } } },
      { kind: "insert", values: { total: "3" } },
    ]);
  });
});
