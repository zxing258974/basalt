// mock 后端的示例表:结构贴近真实业务表,数据用固定种子生成,每次打开都一样。
import type { CellValue, ColumnInfo } from "../types";
import type { MockTable } from "./tableData";

function column(name: string, dataType: string, partial: Partial<ColumnInfo> = {}): ColumnInfo {
  return { name, dataType, isPrimaryKey: false, nullable: true, hasDefault: false, isGenerated: false, valueType: { kind: "text" }, ...partial };
}

const pk = (name: string, dataType: string): ColumnInfo =>
  column(name, dataType, { isPrimaryKey: true, nullable: false, hasDefault: true });

const NULL: CellValue = { kind: "null" };
const int = (n: number): CellValue => ({ kind: "int", value: String(n) });
const text = (value: string): CellValue => ({ kind: "text", value });
const decimal = (n: number): CellValue => ({ kind: "decimal", value: n.toFixed(2) });
const dateTime = (value: string): CellValue => ({ kind: "dateTime", value });

/** 线性同余发生器:同一个种子永远给出同一串数。 */
function random(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state * 1_103_515_245 + 12_345) % 2_147_483_648;
    return state / 2_147_483_648;
  };
}

const NAMES = ["张伟", "王芳", "李娜", "刘洋", "陈静", "杨磊", "赵敏", "黄勇", "周杰", "吴霞", "Emily Chen", "Seán O'Brien"];
const CITIES = ["上海", "北京", "成都", "杭州", "深圳", "武汉", "西安"];
const STATUSES = ["paid", "shipped", "refunded", "pending"];

function pick<T>(next: () => number, list: readonly T[]): T {
  return list[Math.floor(next() * list.length)]!;
}

function timestamp(next: () => number, year = 2026): string {
  const month = 1 + Math.floor(next() * 10);
  const day = 1 + Math.floor(next() * 28);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${year}-${pad(month)}-${pad(day)} ${pad(Math.floor(next() * 24))}:${pad(Math.floor(next() * 60))}:${pad(Math.floor(next() * 60))}`;
}

function customers(): MockTable {
  const next = random(7);
  return {
    columns: [
      pk("id", "int"),
      column("name", "varchar(50)", { nullable: false }),
      column("email", "varchar(120)"),
      column("city", "varchar(30)"),
      column("vip", "tinyint(1)", { hasDefault: true }),
      column("balance", "decimal(10,2)", { hasDefault: true }),
      column("created_at", "datetime", { hasDefault: true }),
      column("note", "text"),
    ],
    rows: Array.from({ length: 320 }, (_, i) => {
      const name = pick(next, NAMES);
      return [
        int(i + 1),
        text(name),
        next() < 0.12 ? NULL : text(`user${i + 1}@example.com`),
        text(pick(next, CITIES)),
        int(next() < 0.2 ? 1 : 0),
        decimal(next() * 5000),
        dateTime(timestamp(next)),
        next() < 0.85 ? NULL : text(i % 2 ? "老客户，\n优先发货" : "需要开发票"),
      ];
    }),
  };
}

function orders(): MockTable {
  const next = random(11);
  return {
    columns: [
      pk("id", "bigint"),
      column("customer_id", "int", { nullable: false }),
      column("amount", "decimal(12,2)", { nullable: false }),
      column("status", "enum('paid','shipped','refunded','pending')", { hasDefault: true }),
      column("placed_at", "datetime", { nullable: false }),
      column("shipping", "json"),
    ],
    rows: Array.from({ length: 1500 }, (_, i) => [
      int(900_719_925_474_000 + i),
      int(1 + Math.floor(next() * 320)),
      decimal(20 + next() * 980),
      text(pick(next, STATUSES)),
      dateTime(timestamp(next)),
      next() < 0.3 ? NULL : { kind: "json", value: `{"city":"${pick(next, CITIES)}","express":"SF"}` },
    ]),
  };
}

function orderItems(): MockTable {
  const next = random(13);
  return {
    columns: [pk("order_id", "bigint"), pk("line", "int"), column("sku", "varchar(20)"), column("qty", "int")],
    rows: Array.from({ length: 60 }, (_, i) => [
      int(900_719_925_474_000 + Math.floor(i / 3)),
      int((i % 3) + 1),
      text(`SKU-${1000 + Math.floor(next() * 40)}`),
      int(1 + Math.floor(next() * 5)),
    ]),
  };
}

function products(): MockTable {
  const next = random(17);
  return {
    columns: [
      pk("id", "int"),
      column("sku", "varchar(20)", { nullable: false }),
      column("title", "varchar(100)"),
      column("price", "decimal(10,2)"),
      column("thumbnail", "blob", { valueType: { kind: "binary" } }),
      column("location", "geometry"),
      column("label", "varchar(120)", { isGenerated: true }),
    ],
    rows: Array.from({ length: 40 }, (_, i) => [
      int(i + 1),
      text(`SKU-${1000 + i}`),
      text(`示例商品 ${i + 1}`),
      decimal(5 + next() * 200),
      next() < 0.5 ? NULL : { kind: "bytes", value: "89504e470d0a1a0a0000000d49484452" },
      { kind: "unsupported", value: { typeName: "GEOMETRY" } },
      text(`SKU-${1000 + i} · 示例商品 ${i + 1}`),
    ]),
  };
}

function dailySales(): MockTable {
  const next = random(19);
  return {
    columns: [column("day", "date"), column("orders", "bigint"), column("total", "decimal(14,2)")],
    rows: Array.from({ length: 30 }, (_, i) => [
      { kind: "date", value: `2026-09-${String(i + 1).padStart(2, "0")}` },
      int(20 + Math.floor(next() * 80)),
      decimal(2000 + next() * 30000),
    ]),
  };
}

function users(): MockTable {
  const next = random(23);
  return {
    columns: [
      column("id", "uuid", { isPrimaryKey: true, nullable: false }),
      column("jsonb", "jsonb", { nullable: false }),
      column("creation_date", "timestamp without time zone"),
      column("created_by", "uuid"),
    ],
    rows: Array.from({ length: 80 }, (_, i) => {
      const hex = (n: number) => Math.floor(n).toString(16).padStart(8, "0");
      const id = `${hex(next() * 2 ** 32)}-${hex(i).slice(4)}-4${hex(next() * 2 ** 32).slice(5)}-a${hex(next() * 2 ** 32).slice(5)}-${hex(next() * 2 ** 32)}${hex(i).slice(4)}`;
      const name = pick(next, NAMES);
      return [
        text(id),
        { kind: "json", value: `{"active": true, "barcode": "${31000000 + i}", "personal": {"lastName": "${name}"}}` },
        dateTime(`${timestamp(next, 2025)}.${String(Math.floor(next() * 999)).padStart(3, "0")}`),
        NULL,
      ];
    }),
  };
}

function notes(): MockTable {
  return {
    columns: [
      column("id", "INTEGER", { isPrimaryKey: true, nullable: false, hasDefault: true }),
      column("body", "TEXT", { nullable: false }),
      column("tags", "TEXT"),
      column("created_at", "TEXT"),
    ],
    rows: [
      [int(1), text("周五前把 M2 做完"), text("工作"), text("2026-10-17 09:12:00")],
      [int(2), text("买菜：西红柿、鸡蛋、葱"), text("生活"), text("2026-10-18 18:40:31")],
      [int(3), text("读书笔记：\n第一章 讲了索引"), NULL, text("2026-10-19 22:05:10")],
    ],
  };
}

function generic(name: string): MockTable {
  const next = random(name.length * 31);
  return {
    columns: [pk("id", "int"), column("name", "varchar(80)"), column("updated_at", "datetime")],
    rows: Array.from({ length: 25 }, (_, i) => [int(i + 1), text(`${name} #${i + 1}`), dateTime(timestamp(next))]),
  };
}

const SEEDS: Record<string, () => MockTable> = {
  customers,
  orders,
  order_items: orderItems,
  products,
  v_daily_sales: dailySales,
  users,
  notes,
};

export function seedTable(name: string): MockTable {
  return (SEEDS[name] ?? (() => generic(name)))();
}
