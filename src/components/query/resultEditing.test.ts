import { describe, expect, it } from "vitest";
import type { ResultColumn, ResultSet } from "../../lib/queryTypes";
import type { ColumnInfo, RowChange, TableRef } from "../../lib/types";
import {
  editPlan,
  gridNames,
  joinedChangeProblem,
  sourceTables,
  statementShape,
  toTableChanges,
  trustedSources,
} from "./resultEditing";

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

const tableNames = (sql: string, dialect: "mysql" | "postgres" | "sqlite" = "mysql") => {
  const shape = statementShape(sql, dialect);
  return shape.kind === "tables" ? shape.tables.map((t) => t.name.map((part) => part.text).join(".")) : shape.kind;
};

describe("statementShape", () => {
  it.each([
    "SELECT * FROM orders",
    "select id, total from shop.orders o where id > 1 order by id limit 10",
    "SELECT `id` FROM `orders` AS `o` FOR UPDATE",
    'SELECT "id" FROM "public"."orders"',
    "-- 说明\nSELECT id FROM orders WHERE note = 'a, b JOIN c' /* , d */",
    "SELECT id, (SELECT count(*) FROM items i WHERE i.order_id = o.id) AS n FROM orders o",
    "SELECT id FROM orders WHERE id IN (SELECT order_id FROM items);",
  ])("reads a single table: %s", (sql) => {
    expect(tableNames(sql)).toHaveLength(1);
  });

  it.each([
    ["SELECT * FROM orders o JOIN customers c ON c.id = o.customer_id", ["orders", "customers"]],
    ["SELECT * FROM orders a, orders b", ["orders", "orders"]],
    ["SELECT * FROM orders NATURAL JOIN customers", ["orders", "customers"]],
    [
      "SELECT * FROM shop.orders AS o LEFT OUTER JOIN items i USING (order_id) INNER JOIN customers c ON (c.id = o.customer_id AND c.vip IN (1, 2)) WHERE o.id > 1",
      ["shop.orders", "items", "customers"],
    ],
    ["SELECT * FROM a JOIN b ON b.id = (SELECT max(id) FROM c WHERE c.x = 1) ORDER BY 1", ["a", "b"]],
    ["SELECT * FROM a STRAIGHT_JOIN b ON a.id = b.id", ["a", "b"]],
  ])("reads every joined table: %s", (sql, names) => {
    expect(tableNames(sql)).toEqual(names);
  });

  it.each([
    "SELECT * FROM (SELECT * FROM orders) t",
    "SELECT * FROM orders o JOIN (SELECT * FROM items) i ON i.order_id = o.id",
    "SELECT * FROM orders o, LATERAL (SELECT 1) x",
    "SELECT * FROM orders o LEFT customers c",
    "SELECT id FROM orders UNION SELECT id FROM orders",
    "SELECT 1",
    "SELECT a.id FROM orders a /*! JOIN orders b ON b.id = a.id */",
  ])("rejects subqueries, set operations and odd FROMs: %s", (sql) => {
    expect(statementShape(sql, "mysql").kind).toBe("multiple");
  });

  it("only SELECT counts", () => {
    expect(statementShape("WITH t AS (SELECT 1) SELECT * FROM t", "postgres").kind).toBe("notSelect");
    expect(statementShape("SHOW TABLES", "mysql").kind).toBe("notSelect");
    expect(statementShape("(SELECT * FROM orders)", "mysql").kind).toBe("notSelect");
  });

  it("knows each dialect's quoting", () => {
    expect(tableNames("SELECT [id] FROM [orders]", "sqlite")).toEqual(["orders"]);
    expect(tableNames("SELECT id FROM orders # , other\nWHERE 1")).toEqual(["orders"]);
    expect(tableNames("SELECT id FROM orders WHERE a = 'it\\'s, x'")).toEqual(["orders"]);
    // PostgreSQL 的 E'…' 和 $tag$ 字符串里的东西不算语句。
    expect(tableNames("SELECT E'\\'', id FROM t", "postgres")).toEqual(["t"]);
    expect(tableNames("SELECT E'\\'' AS q FROM t a JOIN t b ON true", "postgres")).toEqual(["t", "t"]);
    expect(tableNames("SELECT id FROM t WHERE x = $q$ , u JOIN v $q$", "postgres")).toEqual(["t"]);
    expect(tableNames("SELECT id FROM t, u WHERE x = $1", "postgres")).toEqual(["t", "u"]);
  });

  it("returns the table names and aliases as written", () => {
    expect(statementShape('SELECT * FROM "Shop"."Orders" o JOIN items', "postgres")).toEqual({
      kind: "tables",
      tables: [
        {
          name: [
            { text: "Shop", quoted: true },
            { text: "Orders", quoted: true },
          ],
          alias: { text: "o", quoted: false },
        },
        { name: [{ text: "items", quoted: false }], alias: null },
      ],
      items: "star",
      subquery: false,
    });
  });
});

describe("sourceTables", () => {
  const sql = "SELECT * FROM orders";

  it("finds the table every sourced column comes from", () => {
    expect(sourceTables(sql, set(col("id", [ORDERS, "id"]), col("n")), "mysql")).toEqual({
      tables: [ORDERS],
      joined: false,
    });
  });

  it("needs FROM to name the table the columns come from (a view is not its base table)", () => {
    const columns = set(col("id", [ORDERS, "id"]));
    expect(sourceTables("SELECT * FROM shop.ORDERS", columns, "mysql")).toHaveProperty("tables", [ORDERS]);
    expect(sourceTables("SELECT * FROM `orders`", columns, "mysql")).toHaveProperty("tables", [ORDERS]);
    const view = sourceTables("SELECT * FROM v_orders", columns, "mysql") as { reason: string };
    expect(view.reason).toContain("不是 FROM 里写的那张");
    const sqliteView = sourceTables("SELECT * FROM v_orders", columns, "sqlite") as { reason: string };
    expect(sqliteView.reason).toContain("可能是视图");
    expect(sourceTables("SELECT * FROM other_db.orders", columns, "mysql")).toHaveProperty("reason");
    expect(sourceTables("SELECT * FROM `Orders`", columns, "mysql")).toHaveProperty("reason");
    const pg = { database: "folio", schema: "public", table: "orders" };
    const pgSet = set(col("id", [pg, "id"]));
    expect(sourceTables("SELECT * FROM public.orders", pgSet, "postgres")).toHaveProperty("tables", [pg]);
    expect(sourceTables("SELECT * FROM archive.orders", pgSet, "postgres")).toHaveProperty("reason");
  });

  it("offers every joined table that appears once, in FROM order", () => {
    const joined = set(col("name", [OTHER, "name"]), col("id", [ORDERS, "id"]), col("id", [OTHER, "id"]));
    const sqlJoin = "SELECT c.name, o.id, c.id FROM orders o JOIN customers c ON c.id = o.customer_id";
    expect(sourceTables(sqlJoin, joined, "mysql")).toEqual({ tables: [ORDERS, OTHER], joined: true });
    // 自连接:分不清结果里的列是哪一次的。
    const self = sourceTables(
      "SELECT a.id FROM orders a JOIN orders b ON b.id = a.id",
      set(col("id", [ORDERS, "id"])),
      "mysql",
    );
    expect((self as { reason: string }).reason).toContain("出现了不止一次");
    // 一张能改、一张不能(来自视图)时只给能改的。
    const mixed = set(col("id", [ORDERS, "id"]), col("total", [{ ...OTHER, table: "base" }, "total"]));
    expect(sourceTables("SELECT * FROM orders o JOIN v_totals v ON v.id = o.id", mixed, "mysql")).toEqual({
      tables: [ORDERS],
      joined: true,
    });
  });

  it("passes on the backend's reason, such as a temporary table", () => {
    const temp = { ...set(col("id", [ORDERS, "id"])), readOnly: "结果来自临时表 orders" };
    expect(sourceTables("SELECT * FROM orders", temp, "mysql")).toEqual({ reason: "结果来自临时表 orders" });
  });

  it("explains why a result can't be edited", () => {
    const reason = (s: ResultSet, text = sql) => (sourceTables(text, s, "mysql") as { reason: string }).reason;
    expect(reason(set(col("id", [ORDERS, "id"])), "SHOW TABLES")).toContain("只有 SELECT");
    expect(reason(set(col("id", [ORDERS, "id"])), "SELECT * FROM (SELECT 1) t")).toContain("子查询");
    expect(reason(set(col("n")))).toContain("没有直接取自表的列");
    expect(reason(set(col("id", [ORDERS, "id"]), col("again", [ORDERS, "id"])))).toContain("不止一次");
  });
});

describe("SQLite's column sources", () => {
  const EMP: TableRef = { database: "main", schema: null, table: "emp" };

  it("does not trust a subquery's column, which SQLite reports as a table column", () => {
    const sql = "SELECT e.id, (SELECT name FROM emp p WHERE p.id = e.manager_id) AS boss FROM emp e";
    const raw = set(col("id", [EMP, "id"]), col("boss", [EMP, "name"]));
    expect(trustedSources(sql, raw, "sqlite").columns.map((c) => c.source?.column ?? null)).toEqual(["id", null]);
    expect(sourceTables(sql, raw, "sqlite")).toEqual({ tables: [EMP], joined: false });
    const star = "SELECT *, (SELECT name FROM emp p WHERE p.id = e.manager_id) AS boss FROM emp e";
    expect((sourceTables(star, raw, "sqlite") as { reason: string }).reason).toContain("又有 *");
  });

  it("refuses a join with a view over a table that is also in FROM", () => {
    // managers 是 SELECT id AS mid, name AS mname FROM emp 的视图:它的列 SQLite 也报成 emp 的。
    const sql = "SELECT m.mname, e.id, e.salary FROM managers m JOIN emp e ON e.manager_id = m.mid";
    const raw = set(col("mname", [EMP, "name"]), col("id", [EMP, "id"]), col("salary", [EMP, "salary"]));
    expect((sourceTables(sql, raw, "sqlite") as { reason: string }).reason).toContain("managers");
  });

  it("keeps plain columns, aliases and DISTINCT", () => {
    const sql = "SELECT DISTINCT o.id, total AS t, `note` FROM orders o";
    const raw = set(col("id", [ORDERS, "id"]), col("t", [ORDERS, "total"]), col("note", [ORDERS, "note"]));
    expect(trustedSources(sql, raw, "mysql")).toBe(raw);
  });
});

describe("gridNames", () => {
  it("tells same-named columns apart by their table, then by position", () => {
    const columns = [
      col("id", [ORDERS, "id"]),
      col("name", [OTHER, "name"]),
      col("id", [OTHER, "id"]),
      col("n"),
      col("n"),
    ];
    expect(gridNames(columns)).toEqual(["id（orders）", "name", "id（customers）", "n", "n #5"]);
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
    expect(plan.rowEdits).toBe(true);
  });

  it("edits one table of a join, cells only", () => {
    const joined = set(col("id", [ORDERS, "id"]), col("total", [ORDERS, "total"]), col("id", [OTHER, "id"]));
    const plan = editPlan(joined, ORDERS, table, true);
    expect(plan.kind).toBe("editable");
    if (plan.kind !== "editable") return;
    expect(plan.columns.map((c) => [c.name, c.isGenerated])).toEqual([
      ["id（orders）", false],
      ["total", false],
      ["id（customers）", true],
    ]);
    expect(plan.tableColumn).toEqual({ "id（orders）": "id", total: "total" });
    expect(plan.rowEdits).toBe(false);
  });

  it("needs a primary key, and all of it in the result", () => {
    const noKey = editPlan(set(col("id", [ORDERS, "id"])), ORDERS, [info("id")]);
    expect(noKey).toEqual({ kind: "readOnly", reason: "orders 没有主键，不能在这里改。" });
    const twoKeys = [info("a", { isPrimaryKey: true }), info("b", { isPrimaryKey: true })];
    const partial = editPlan(set(col("a", [ORDERS, "a"])), ORDERS, twoKeys);
    expect(partial).toEqual({ kind: "readOnly", reason: "结果里没有主键列 b，不能在这里改。" });
    // 连接时别的表的同名主键不算。
    const joined = editPlan(set(col("id", [OTHER, "id"]), col("total", [ORDERS, "total"])), ORDERS, table, true);
    expect(joined).toEqual({ kind: "readOnly", reason: "结果里没有 orders 的主键列 id，不能在这里改。" });
  });
});

describe("joinedChangeProblem", () => {
  const update = (id: string | null, values: Record<string, string>): RowChange => ({
    kind: "update",
    key: { id: id === null ? { kind: "null" } : { kind: "int", value: id } },
    values,
  });

  it("refuses rows the outer join did not match", () => {
    expect(joinedChangeProblem([update(null, { total: "1" })], ORDERS)).toContain("没有对应的行");
  });

  it("refuses changing the key of a row that shows up more than once", () => {
    const moved = [update("1", { id: "9" }), update("1", { note: "x" })];
    expect(joinedChangeProblem(moved, ORDERS)).toContain("又改了它的主键");
    expect(joinedChangeProblem([update("1", { id: "9" })], ORDERS)).toBeNull();
  });

  it("refuses the same row changed to two different values", () => {
    const same = [update("1", { total: "1" }), update("1", { note: "x" })];
    expect(joinedChangeProblem(same, ORDERS)).toBeNull();
    expect(joinedChangeProblem([...same, update("1", { total: "2" })], ORDERS)).toContain("「total」改成了不同的值");
    expect(joinedChangeProblem([...same, update("1", { total: "1" })], ORDERS)).toBeNull();
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
