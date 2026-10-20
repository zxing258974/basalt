// 设计器的草稿编辑:全是纯函数,返回新的 TableDesign,不改原对象。
import type { ColumnDef, ForeignKeyDef, IndexDef, TableDesign, TableOptions } from "../../lib/designTypes";
import type { DriverKind } from "../../lib/types";

let counter = 0;

/** 设计器里新加的字段、索引、外键的 id;不带 `col:` 这类前缀,后端据此认出是新的。 */
export function newId(): string {
  counter += 1;
  return `new:${counter}`;
}

export const isMysql = (driver: DriverKind) => driver === "mysql" || driver === "mariadb";

/** 常用类型,按方言给下拉建议;也可以直接手填。 */
export function typeSuggestions(driver: DriverKind): string[] {
  if (isMysql(driver)) {
    return [
      "int",
      "bigint",
      "tinyint",
      "smallint",
      "decimal(10,2)",
      "float",
      "double",
      "varchar(255)",
      "char(32)",
      "text",
      "mediumtext",
      "longtext",
      "json",
      "date",
      "datetime",
      "timestamp",
      "time",
      "year",
      "blob",
      "varbinary(255)",
      "bit(1)",
      "enum('a','b')",
    ];
  }
  if (driver === "postgres") {
    return [
      "integer",
      "bigint",
      "smallint",
      "numeric(10,2)",
      "real",
      "double precision",
      "boolean",
      "varchar(255)",
      "text",
      "char(32)",
      "uuid",
      "jsonb",
      "json",
      "date",
      "timestamp",
      "timestamptz",
      "time",
      "interval",
      "bytea",
      "inet",
      "text[]",
    ];
  }
  return ["INTEGER", "TEXT", "REAL", "NUMERIC", "BLOB", "BOOLEAN", "DATETIME", "VARCHAR(255)"];
}

/** 索引类型下拉:第一项 null 是默认的 B 树。 */
export function indexMethods(driver: DriverKind): (string | null)[] {
  if (isMysql(driver)) return [null, "FULLTEXT", "SPATIAL", "HASH"];
  if (driver === "postgres") return [null, "hash", "gin", "gist", "brin", "spgist"];
  return [null];
}

function column(name: string, dataType: string): ColumnDef {
  return {
    id: newId(),
    name,
    dataType,
    nullable: true,
    default: null,
    autoIncrement: false,
    comment: null,
    collation: null,
    onUpdate: null,
    generated: null,
  };
}

/** 新字段的默认类型。 */
export function defaultType(driver: DriverKind): string {
  if (isMysql(driver)) return "varchar(255)";
  return driver === "postgres" ? "text" : "TEXT";
}

/** 新建表的起点:一个自增主键 id。 */
export function newTableDesign(driver: DriverKind, database: string, schema: string | null): TableDesign {
  const idType = isMysql(driver) ? "int" : driver === "postgres" ? "integer" : "INTEGER";
  const id: ColumnDef = { ...column("id", idType), nullable: false, autoIncrement: true };
  return {
    table: { database, schema, table: "" },
    columns: [id],
    primaryKey: [id.id],
    primaryKeyName: null,
    indexes: [],
    foreignKeys: [],
    checks: [],
    triggers: [],
    options: {
      engine: isMysql(driver) ? "InnoDB" : null,
      charset: null,
      collation: null,
      comment: null,
      withoutRowid: false,
      strict: false,
    },
    version: "",
  };
}

export function renameTable(design: TableDesign, name: string): TableDesign {
  return { ...design, table: { ...design.table, table: name } };
}

/** 在 `at` 处插入一个空白字段(省略则加在最后),返回新结构和新字段的 id。 */
export function addColumn(design: TableDesign, driver: DriverKind, at?: number): [TableDesign, string] {
  const added = column("", defaultType(driver));
  const index = at ?? design.columns.length;
  const columns = [...design.columns.slice(0, index), added, ...design.columns.slice(index)];
  return [{ ...design, columns }, added.id];
}

export function updateColumn(design: TableDesign, id: string, patch: Partial<ColumnDef>): TableDesign {
  return { ...design, columns: design.columns.map((c) => (c.id === id ? { ...c, ...patch } : c)) };
}

/** 删字段时一并从主键、索引、外键里去掉;索引、外键空了就整个删掉。 */
export function removeColumn(design: TableDesign, id: string): TableDesign {
  const indexes = design.indexes
    .map((i) => ({ ...i, columns: i.columns.filter((c) => c.column !== id) }))
    .filter((i) => i.definition !== null || i.columns.length > 0);
  const foreignKeys = design.foreignKeys
    .map((f) => {
      const keep = f.columns.map((c, n) => [c, n] as const).filter(([c]) => c !== id);
      return {
        ...f,
        columns: keep.map(([c]) => c),
        refColumns: f.refColumns.length ? keep.map(([, n]) => f.refColumns[n] ?? "") : [],
      };
    })
    .filter((f) => f.columns.length > 0);
  return {
    ...design,
    columns: design.columns.filter((c) => c.id !== id),
    primaryKey: design.primaryKey.filter((c) => c !== id),
    indexes,
    foreignKeys,
  };
}

/** 上移(-1)或下移(+1)一格;到头了原样返回。 */
export function moveColumn(design: TableDesign, id: string, delta: -1 | 1): TableDesign {
  const from = design.columns.findIndex((c) => c.id === id);
  const to = from + delta;
  if (from < 0 || to < 0 || to >= design.columns.length) return design;
  const columns = [...design.columns];
  [columns[from], columns[to]] = [columns[to]!, columns[from]!];
  return { ...design, columns };
}

/** 加入主键时字段改成非空;移出时保持原样。 */
export function togglePrimaryKey(design: TableDesign, id: string): TableDesign {
  if (design.primaryKey.includes(id)) {
    return { ...design, primaryKey: design.primaryKey.filter((c) => c !== id) };
  }
  return { ...updateColumn(design, id, { nullable: false }), primaryKey: [...design.primaryKey, id] };
}

export function addIndex(design: TableDesign): TableDesign {
  const used = new Set(design.indexes.map((i) => i.name));
  let n = design.indexes.length + 1;
  const base = design.table.table || "table";
  while (used.has(`idx_${base}_${n}`)) n += 1;
  const index: IndexDef = {
    id: newId(),
    name: `idx_${base}_${n}`,
    columns: [],
    unique: false,
    method: null,
    definition: null,
    isConstraint: false,
  };
  return { ...design, indexes: [...design.indexes, index] };
}

export function updateIndex(design: TableDesign, id: string, patch: Partial<IndexDef>): TableDesign {
  return { ...design, indexes: design.indexes.map((i) => (i.id === id ? { ...i, ...patch } : i)) };
}

export function removeIndex(design: TableDesign, id: string): TableDesign {
  return { ...design, indexes: design.indexes.filter((i) => i.id !== id) };
}

export function addForeignKey(design: TableDesign): TableDesign {
  const fk: ForeignKeyDef = {
    id: newId(),
    name: "",
    columns: [],
    refSchema: null,
    refTable: "",
    refColumns: [],
    onUpdate: "noAction",
    onDelete: "noAction",
  };
  return { ...design, foreignKeys: [...design.foreignKeys, fk] };
}

export function updateForeignKey(design: TableDesign, id: string, patch: Partial<ForeignKeyDef>): TableDesign {
  return { ...design, foreignKeys: design.foreignKeys.map((f) => (f.id === id ? { ...f, ...patch } : f)) };
}

export function removeForeignKey(design: TableDesign, id: string): TableDesign {
  return { ...design, foreignKeys: design.foreignKeys.filter((f) => f.id !== id) };
}

export function updateOptions(design: TableDesign, patch: Partial<TableOptions>): TableDesign {
  return { ...design, options: { ...design.options, ...patch } };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export type ColumnStatus = "added" | "edited" | "unchanged";

export function columnStatus(original: TableDesign | null, column: ColumnDef): ColumnStatus {
  const before = original?.columns.find((c) => c.id === column.id);
  if (!before) return "added";
  return same(before, column) ? "unchanged" : "edited";
}

function listChanges<T extends { id: string }>(before: T[], after: T[]): number {
  const removed = before.filter((b) => !after.some((a) => a.id === b.id)).length;
  const changed = after.filter(
    (a) =>
      !same(
        before.find((b) => b.id === a.id),
        a,
      ),
  ).length;
  return removed + changed;
}

/** 草稿相对原结构改了几处(字段、索引、外键各算一处,主键、选项、表名各算一处)。新表恒为 1。 */
export function changeCount(original: TableDesign | null, draft: TableDesign): number {
  if (!original) return 1;
  const order = (d: TableDesign) =>
    d.columns.map((c) => c.id).filter((id) => original.columns.some((c) => c.id === id));
  return (
    listChanges(original.columns, draft.columns) +
    (same(order(original), order(draft)) ? 0 : 1) +
    (same(original.primaryKey, draft.primaryKey) ? 0 : 1) +
    listChanges(original.indexes, draft.indexes) +
    listChanges(original.foreignKeys, draft.foreignKeys) +
    (same(original.options, draft.options) ? 0 : 1) +
    (original.table.table === draft.table.table ? 0 : 1)
  );
}

/**
 * PostgreSQL 不能调整字段顺序:原有字段不能挪,新字段只能在新字段之间挪。
 * 返回能不能把 `id` 挪 `delta` 格。
 */
export function canMove(
  driver: DriverKind,
  original: TableDesign | null,
  draft: TableDesign,
  id: string,
  delta: -1 | 1,
) {
  const from = draft.columns.findIndex((c) => c.id === id);
  const target = draft.columns[from + delta];
  if (from < 0 || !target) return false;
  if (driver !== "postgres" || !original) return true;
  const existing = (columnId: string) => original.columns.some((c) => c.id === columnId);
  return !existing(id) && !existing(target.id);
}

/**
 * 三方合并一个列表:`before` 是草稿当初的基准,`after` 是重新读到的结构。
 * 用户没动过的跟着新结构走;用户改过的保留用户的;别人加的插在它在新结构里的前一项后面;
 * 别人删掉、用户又改过的,`reissue` 时换个新 id 当成新加的(索引、外键),否则原样留着让后端报出来。
 */
function mergeList<T extends { id: string }>(before: T[], after: T[], draft: T[], reissue: boolean): T[] {
  const result: T[] = [];
  for (const item of draft) {
    const old = before.find((b) => b.id === item.id);
    const fresh = after.find((a) => a.id === item.id);
    if (!old) result.push(item);
    else if (fresh) result.push(same(item, old) ? fresh : item);
    else if (!same(item, old)) result.push(reissue ? { ...item, id: newId() } : item);
  }
  after.forEach((item, index) => {
    if (before.some((b) => b.id === item.id) || result.some((r) => r.id === item.id)) return;
    const previous = after[index - 1];
    const at = previous ? result.findIndex((r) => r.id === previous.id) : -1;
    result.splice(previous && at < 0 ? result.length : at + 1, 0, item);
  });
  return result;
}

/** 把草稿挪到重新读到的结构上,保留用户的修改,不把别人新加的东西当成用户删掉的。 */
export function rebase(before: TableDesign, after: TableDesign, draft: TableDesign): TableDesign {
  const pick = <T>(mine: T, base: T, theirs: T) => (same(mine, base) ? theirs : mine);
  return {
    ...draft,
    table: { ...draft.table, table: pick(draft.table.table, before.table.table, after.table.table) },
    columns: mergeList(before.columns, after.columns, draft.columns, false),
    primaryKey: pick(draft.primaryKey, before.primaryKey, after.primaryKey),
    primaryKeyName: pick(draft.primaryKeyName, before.primaryKeyName, after.primaryKeyName),
    indexes: mergeList(before.indexes, after.indexes, draft.indexes, true),
    foreignKeys: mergeList(before.foreignKeys, after.foreignKeys, draft.foreignKeys, true),
    checks: after.checks,
    triggers: after.triggers,
    options: pick(draft.options, before.options, after.options),
    version: after.version,
  };
}
