// mock 后端的表结构设计:内存里的表结构,DDL 只做个大概——够界面开发和交互测试用,
// 真正的方言细节在 Rust 端。行为上贴近后端:按 id 配对、预览什么执行什么、建表改名后对象列表跟着变。
import type { ColumnDef, DdlOutcome, DdlPreview, IndexDef, TableDesign } from "../designTypes";
import type { ColumnInfo, CommandError, DriverKind, TableInfo, TableRef } from "../types";
import { seedTable } from "./seedTables";

function fail(code: string, message: string): never {
  const error: CommandError = { code, message, fields: [] };
  throw error;
}

const isMysql = (driver: DriverKind) => driver === "mysql" || driver === "mariadb";
const quote = (driver: DriverKind, name: string) =>
  isMysql(driver) ? `\`${name.replaceAll("`", "``")}\`` : `"${name.replaceAll('"', '""')}"`;
const literal = (text: string) => `'${text.replaceAll("'", "''")}'`;

function qualified(driver: DriverKind, table: TableRef): string {
  const first = driver === "postgres" ? (table.schema ?? "") : table.database;
  return `${quote(driver, first)}.${quote(driver, table.table)}`;
}

const keyOf = (id: string, table: TableRef) => [id, table.database, table.schema ?? "", table.table].join("|");
const scopeOf = (id: string, database: string, schema: string | null) => [id, database, schema ?? ""].join("|");

/** 种子表的「读出来的结构」。 */
function seedDefault(column: ColumnInfo): string | null {
  if (!column.hasDefault || column.isPrimaryKey) return null;
  if (/date|time/i.test(column.dataType)) return "CURRENT_TIMESTAMP";
  if (/enum/i.test(column.dataType)) return "'pending'";
  return /int|decimal/i.test(column.dataType) ? "0" : "''";
}

function fromSeed(driver: DriverKind, table: TableRef): TableDesign {
  const seed = seedTable(table.table).columns;
  const keys = seed.filter((c) => c.isPrimaryKey);
  const columns: ColumnDef[] = seed.map((c) => ({
    id: `col:${c.name}`,
    name: c.name,
    dataType: c.dataType,
    nullable: c.nullable,
    default: seedDefault(c),
    autoIncrement: keys.length === 1 && c.isPrimaryKey && c.hasDefault && /int/i.test(c.dataType),
    comment: null,
    collation: null,
    onUpdate: null,
    generated: c.isGenerated ? { expression: "concat(sku, ' · ', title)", stored: false } : null,
  }));
  const index = (name: string, column: string, unique = false): IndexDef => ({
    id: `idx:${name}`,
    name,
    columns: [{ column: `col:${column}`, descending: false, length: null }],
    unique,
    method: null,
    definition: null,
    isConstraint: false,
  });
  const has = (name: string) => columns.some((c) => c.name === name);
  return {
    table,
    columns,
    primaryKey: keys.map((c) => `col:${c.name}`),
    primaryKeyName: driver === "postgres" && keys.length ? `${table.table}_pkey` : null,
    indexes: [
      ...(has("email") ? [index(`uk_${table.table}_email`, "email", true)] : []),
      ...(has("city") ? [index(`idx_${table.table}_city`, "city")] : []),
    ],
    foreignKeys: has("customer_id")
      ? [
          {
            id: "fk:fk_orders_customer",
            name: "fk_orders_customer",
            columns: ["col:customer_id"],
            refSchema: null,
            refTable: "customers",
            refColumns: ["id"],
            onUpdate: "noAction",
            onDelete: "restrict",
          },
        ]
      : [],
    checks: [],
    triggers: [],
    options: {
      engine: isMysql(driver) ? "InnoDB" : null,
      charset: isMysql(driver) ? "utf8mb4" : null,
      collation: isMysql(driver) ? "utf8mb4_0900_ai_ci" : null,
      comment: null,
      withoutRowid: false,
      strict: false,
    },
  };
}

function validate(driver: DriverKind, design: TableDesign): void {
  const invalid = (message: string) => fail("invalid_edit", message);
  if (!design.table.table.trim()) invalid("请填写表名。");
  if (design.columns.length === 0) invalid("表里至少要有一个字段。");
  const seen = new Set<string>();
  design.columns.forEach((column, index) => {
    if (!column.name.trim()) invalid(`第 ${index + 1} 个字段没有填字段名。`);
    const key = driver === "postgres" ? column.name : column.name.toLowerCase();
    if (seen.has(key)) invalid(`字段名「${column.name}」重复了。`);
    seen.add(key);
    if (driver !== "sqlite" && !column.dataType.trim()) invalid(`字段「${column.name}」没有填类型。`);
  });
  for (const index of design.indexes) {
    if (!index.definition && !index.name.trim()) invalid("有索引没有填索引名。");
    if (!index.definition && index.columns.length === 0) invalid(`索引「${index.name}」至少选一个字段。`);
  }
  design.foreignKeys.forEach((fk, i) => {
    const label = fk.name ? `外键「${fk.name}」` : `第 ${i + 1} 个外键`;
    if (fk.columns.length === 0) invalid(`${label}至少选一个字段。`);
    if (!fk.refTable.trim()) invalid(`${label}没有选引用的表。`);
    if (fk.refColumns.length !== fk.columns.length) invalid(`${label}的字段和引用的字段个数不一致。`);
  });
}

function columnSql(driver: DriverKind, column: ColumnDef): string {
  const parts = [quote(driver, column.name), column.dataType];
  if (!column.nullable) parts.push("NOT NULL");
  if (column.default !== null) parts.push(`DEFAULT ${column.default}`);
  if (column.autoIncrement) parts.push(isMysql(driver) ? "AUTO_INCREMENT" : "GENERATED BY DEFAULT AS IDENTITY");
  if (column.comment && isMysql(driver)) parts.push(`COMMENT ${literal(column.comment)}`);
  return parts.filter(Boolean).join(" ");
}

function nameOf(design: TableDesign, id: string): string {
  return design.columns.find((c) => c.id === id)?.name ?? "";
}

function indexSql(driver: DriverKind, design: TableDesign, index: IndexDef): string {
  const columns = index.columns.map((c) => quote(driver, nameOf(design, c.column)) + (c.descending ? " DESC" : ""));
  const unique = index.unique ? "UNIQUE " : "";
  return `CREATE ${unique}INDEX ${quote(driver, index.name)} ON ${qualified(driver, design.table)} (${columns.join(", ")})`;
}

function createTable(driver: DriverKind, design: TableDesign): string[] {
  const lines = design.columns.map((c) => columnSql(driver, c));
  if (design.primaryKey.length) {
    lines.push(`PRIMARY KEY (${design.primaryKey.map((id) => quote(driver, nameOf(design, id))).join(", ")})`);
  }
  return [
    `CREATE TABLE ${qualified(driver, design.table)} (\n  ${lines.join(",\n  ")}\n)`,
    ...design.indexes.map((i) => indexSql(driver, design, i)),
  ];
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

function alterTable(driver: DriverKind, before: TableDesign, after: TableDesign): string[] {
  const actions: string[] = [];
  const kept = new Set(after.columns.map((c) => c.id));
  for (const column of before.columns) {
    if (!kept.has(column.id)) actions.push(`DROP COLUMN ${quote(driver, column.name)}`);
  }
  for (const column of after.columns) {
    const old = before.columns.find((c) => c.id === column.id);
    if (!old) actions.push(`ADD COLUMN ${columnSql(driver, column)}`);
    else if (old.name !== column.name)
      actions.push(`RENAME COLUMN ${quote(driver, old.name)} TO ${quote(driver, column.name)}`);
    if (old && !same({ ...old, id: "", name: "" }, { ...column, id: "", name: "" }))
      actions.push(`MODIFY COLUMN ${columnSql(driver, column)}`);
  }
  if (!same(before.primaryKey, after.primaryKey)) {
    const names = after.primaryKey.map((id) => quote(driver, nameOf(after, id)));
    actions.push(names.length ? `ADD PRIMARY KEY (${names.join(", ")})` : "DROP PRIMARY KEY");
  }
  const indexes = (list: IndexDef[]) => new Map(list.map((i) => [i.id, i]));
  const oldIndexes = indexes(before.indexes);
  for (const index of before.indexes) {
    if (!after.indexes.some((i) => same(i, index))) actions.push(`DROP INDEX ${quote(driver, index.name)}`);
  }
  for (const fk of before.foreignKeys) {
    if (!after.foreignKeys.some((f) => same(f, fk))) actions.push(`DROP FOREIGN KEY ${quote(driver, fk.name)}`);
  }
  for (const fk of after.foreignKeys.filter((f) => !before.foreignKeys.some((b) => same(b, f)))) {
    const columns = fk.columns.map((id) => quote(driver, nameOf(after, id))).join(", ");
    const refs = fk.refColumns.map((c) => quote(driver, c)).join(", ");
    actions.push(`ADD FOREIGN KEY (${columns}) REFERENCES ${quote(driver, fk.refTable)} (${refs})`);
  }
  if ((before.options.comment ?? "") !== (after.options.comment ?? "")) {
    actions.push(`COMMENT = ${literal(after.options.comment ?? "")}`);
  }
  if (before.table.table !== after.table.table) actions.push(`RENAME TO ${quote(driver, after.table.table)}`);
  const table = qualified(driver, before.table);
  const statements = isMysql(driver)
    ? actions.length
      ? [`ALTER TABLE ${table}\n  ${actions.join(",\n  ")}`]
      : []
    : actions.map((action) => `ALTER TABLE ${table} ${action}`);
  const created = after.indexes.filter((i) => !same(oldIndexes.get(i.id), i));
  return [...statements, ...created.map((i) => indexSql(driver, { ...after, table: before.table }, i))];
}

export function createMockDesigns() {
  const designs = new Map<string, TableDesign>();
  /** 每个库(schema)里新建、改名、删掉的表,对象列表据此调整。 */
  const added = new Map<string, string[]>();
  const removed = new Map<string, string[]>();

  const read = (id: string, driver: DriverKind, table: TableRef) =>
    structuredClone(designs.get(keyOf(id, table)) ?? fromSeed(driver, table));

  const plan = (id: string, driver: DriverKind, original: TableRef | null, design: TableDesign) => {
    validate(driver, design);
    const statements = original ? alterTable(driver, read(id, driver, original), design) : createTable(driver, design);
    const mode: DdlPreview["mode"] = isMysql(driver) ? "autocommit" : "transaction";
    return { statements, mode };
  };

  return {
    read,
    preview: plan,
    apply(id: string, driver: DriverKind, original: TableRef | null, design: TableDesign, expected: string[]) {
      const { statements } = plan(id, driver, original, design);
      if (!same(statements, expected)) {
        fail("invalid_edit", "表结构在预览之后变了，要执行的 SQL 和预览的不一样。请重新预览后再保存。");
      }
      const scope = scopeOf(id, design.table.database, design.table.schema);
      if (original) {
        designs.delete(keyOf(id, original));
        removed.set(scope, [...(removed.get(scope) ?? []), original.table]);
      }
      // 新字段落库后变成「读出来的」字段:id 换成按名字的形式。
      const ids = new Map(design.columns.map((c) => [c.id, `col:${c.name}`]));
      const stored: TableDesign = {
        ...structuredClone(design),
        columns: design.columns.map((c) => ({ ...c, id: `col:${c.name}` })),
        primaryKey: design.primaryKey.map((c) => ids.get(c) ?? c),
        indexes: design.indexes.map((i) => ({
          ...i,
          id: `idx:${i.name}`,
          columns: i.columns.map((c) => ({ ...c, column: ids.get(c.column) ?? c.column })),
        })),
        foreignKeys: design.foreignKeys.map((f) => ({
          ...f,
          id: `fk:${f.name}`,
          columns: f.columns.map((c) => ids.get(c) ?? c),
        })),
      };
      designs.set(keyOf(id, design.table), stored);
      removed.set(
        scope,
        (removed.get(scope) ?? []).filter((t) => t !== design.table.table),
      );
      added.set(scope, [...(added.get(scope) ?? []).filter((t) => t !== original?.table), design.table.table]);
      const outcome: DdlOutcome = { statements: statements.length, table: design.table };
      return outcome;
    },
    /** 对象列表叠加建表、改名的结果。 */
    tables(id: string, database: string, schema: string | null, base: TableInfo[]): TableInfo[] {
      const scope = scopeOf(id, database, schema);
      const gone = new Set(removed.get(scope) ?? []);
      const kept = base.filter((t) => !gone.has(t.name));
      const extra = (added.get(scope) ?? []).filter((name) => !kept.some((t) => t.name === name));
      const blank = { engine: null, rows: 0, dataBytes: null, indexBytes: null, createdAt: null, updatedAt: null };
      const extras = [...extra].sort((a, b) => a.localeCompare(b));
      return [...kept, ...extras.map((name): TableInfo => ({ ...blank, name, kind: "table", comment: null }))];
    },
  };
}
