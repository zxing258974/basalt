// mock 后端的表数据:内存里的几张表,行为尽量贴近 Rust 端——
// 显式列、主键排序、多取一行判断下一页、整批成功或整批回滚。
import type {
  ApplyOutcome,
  CellValue,
  ColumnInfo,
  CommandError,
  DriverKind,
  EditInput,
  PageRequest,
  RowChange,
  TablePage,
  TableRef,
} from "../types";
import { seedTable } from "./seedTables";

export interface MockTable {
  columns: ColumnInfo[];
  rows: CellValue[][];
}

function fail(code: string, message: string): never {
  const error: CommandError = { code, message, fields: [] };
  throw error;
}

const quote = (driver: DriverKind, name: string) =>
  driver === "mysql" || driver === "mariadb" ? `\`${name.replaceAll("`", "``")}\`` : `"${name.replaceAll('"', '""')}"`;

function tableName(driver: DriverKind, table: TableRef): string {
  const first = driver === "postgres" ? (table.schema ?? "") : table.database;
  return `${quote(driver, first)}.${quote(driver, table.table)}`;
}

const NUMERIC_TYPE = /int|decimal|numeric|double|float|real|serial/i;

function literal(column: ColumnInfo | undefined, value: EditInput | CellValue): string {
  if (value === null) return "NULL";
  if (typeof value !== "string") {
    if (value.kind === "null") return "NULL";
    if (value.kind === "unsupported") return "NULL";
    return literal(column, String(value.value));
  }
  if (column && NUMERIC_TYPE.test(column.dataType) && /^-?\d+(\.\d+)?$/.test(value.trim())) return value.trim();
  return `'${value.replaceAll("'", "''")}'`;
}

function toCell(column: ColumnInfo, input: EditInput): CellValue {
  if (input === null) return { kind: "null" };
  if (/decimal|numeric/i.test(column.dataType)) return { kind: "decimal", value: input.trim() };
  if (NUMERIC_TYPE.test(column.dataType)) {
    if (!/^-?\d+$/.test(input.trim())) fail("invalid_edit", `「${column.name}」不是有效的整数：${input}`);
    return { kind: "int", value: input.trim() };
  }
  if (/blob|bytea|binary/i.test(column.dataType) && input.startsWith("0x"))
    return { kind: "bytes", value: input.slice(2) };
  if (/datetime|timestamp/i.test(column.dataType)) return { kind: "dateTime", value: input };
  return { kind: "text", value: input };
}

const sameCell = (a: CellValue | undefined, b: CellValue) => JSON.stringify(a) === JSON.stringify(b);

/** 只认 `列 运算符 值`,可用 AND 连接;够在浏览器里调界面。 */
function rowFilter(columns: ColumnInfo[], filter: string): (row: CellValue[]) => boolean {
  if (filter.includes(";"))
    fail("invalid_filter", "筛选条件里不能有分号：这里只填 WHERE 后面的条件，不能再接别的语句。");
  const tests = filter.split(/\s+AND\s+/i).map((part) => {
    const match = /^\s*(\w+)\s*(=|!=|<>|>=|<=|>|<|LIKE|IS NOT NULL|IS NULL)\s*(.*?)\s*$/i.exec(part);
    const index = match ? columns.findIndex((c) => c.name === match[1]) : -1;
    if (!match || index < 0) fail("invalid_filter", "mock 后端只支持「列 运算符 值」形式的筛选，可用 AND 连接。");
    const op = match[2]!.toUpperCase();
    const raw = match[3]!.replace(/^'(.*)'$/, "$1");
    return (row: CellValue[]) => {
      const cell = row[index]!;
      if (op === "IS NULL") return cell.kind === "null";
      if (op === "IS NOT NULL") return cell.kind !== "null";
      if (cell.kind === "null" || cell.kind === "unsupported") return false;
      const text = String(cell.value);
      if (op === "LIKE") return new RegExp(`^${raw.replaceAll("%", ".*").replaceAll("_", ".")}$`, "i").test(text);
      const [a, b] = /^-?\d+(\.\d+)?$/.test(raw) ? [Number(text), Number(raw)] : [text, raw];
      return (
        { "=": a === b, "!=": a !== b, "<>": a !== b, ">": a > b, "<": a < b, ">=": a >= b, "<=": a <= b }[op] ?? false
      );
    };
  });
  return (row) => tests.every((test) => test(row));
}

/** 排序用的文本:不支持的类型按类型名排,布尔按 true/false。 */
function sortText(cell: Exclude<CellValue, { kind: "null" }>): string {
  if (cell.kind === "unsupported") return cell.value.typeName;
  return typeof cell.value === "string" ? cell.value : String(cell.value);
}

function compareCells(a: CellValue, b: CellValue): number {
  if (a.kind === "null" || b.kind === "null") return a.kind === b.kind ? 0 : a.kind === "null" ? -1 : 1;
  const x = sortText(a);
  const y = sortText(b);
  if (typeof x === "string" && typeof y === "string" && /^-?\d/.test(x) && /^-?\d/.test(y))
    return Number(x) - Number(y);
  return String(x).localeCompare(String(y), "zh-CN");
}

export function createMockTables() {
  const tables = new Map<string, MockTable>();
  const keyOf = (connectionId: string, table: TableRef) =>
    [connectionId, table.database, table.schema ?? "", table.table].join("\u0000");
  const get = (connectionId: string, table: TableRef) => {
    const key = keyOf(connectionId, table);
    const existing = tables.get(key) ?? seedTable(table.table);
    tables.set(key, existing);
    return existing;
  };

  function selectSql(driver: DriverKind, table: TableRef, data: MockTable, request: PageRequest): string {
    const pk = data.columns.filter((c) => c.isPrimaryKey).map((c) => quote(driver, c.name));
    const order = request.sort
      ? [`${quote(driver, request.sort.column)}${request.sort.descending ? " DESC" : ""}`, ...pk]
      : pk;
    return [
      `SELECT ${data.columns.map((c) => quote(driver, c.name)).join(", ")} FROM ${tableName(driver, table)}`,
      request.filter?.trim() ? ` WHERE (\n${request.filter.trim()}\n)` : "",
      order.length ? ` ORDER BY ${order.join(", ")}` : "",
      ` LIMIT ${request.limit + 1} OFFSET ${request.offset}`,
    ].join("");
  }

  function statements(driver: DriverKind, table: TableRef, data: MockTable, changes: RowChange[]): string[] {
    if (!data.columns.some((c) => c.isPrimaryKey)) {
      fail("read_only", "这张表没有主键，无法安全地定位要修改的行，只能查看。");
    }
    const column = (name: string) => data.columns.find((c) => c.name === name);
    const where = (key: Record<string, CellValue>) =>
      ` WHERE ${Object.entries(key)
        .map(([k, v]) => `${quote(driver, k)} = ${literal(column(k), v)}`)
        .join(" AND ")}`;
    const rank = { delete: 0, update: 1, insert: 2 } as const;
    return [...changes]
      .sort((a, b) => rank[a.kind] - rank[b.kind])
      .flatMap((change) => {
        const target = tableName(driver, table);
        if (change.kind === "delete") return [`DELETE FROM ${target}${where(change.key)}`];
        const entries = Object.entries(change.values);
        if (change.kind === "update") {
          if (!entries.length) return [];
          const sets = entries.map(([k, v]) => `${quote(driver, k)} = ${literal(column(k), v)}`).join(", ");
          return [`UPDATE ${target} SET ${sets}${where(change.key)}`];
        }
        if (!entries.length) return [`INSERT INTO ${target} DEFAULT VALUES`];
        const names = entries.map(([k]) => quote(driver, k)).join(", ");
        return [
          `INSERT INTO ${target} (${names}) VALUES (${entries.map(([k, v]) => literal(column(k), v)).join(", ")})`,
        ];
      });
  }

  /** 先在副本上全部做完,任何一处失败都不改动原表。 */
  function applyAll(data: MockTable, changes: RowChange[]): CellValue[][] {
    let rows = data.rows.map((row) => [...row]);
    const pk = data.columns.flatMap((c, i) => (c.isPrimaryKey ? [i] : []));
    const find = (key: Record<string, CellValue>) =>
      rows.findIndex((row) => pk.every((i) => sameCell(key[data.columns[i]!.name], row[i]!)));
    const rank = { delete: 0, update: 1, insert: 2 } as const;
    const ordered = [...changes]
      .filter((change) => change.kind !== "update" || Object.keys(change.values).length > 0)
      .sort((a, b) => rank[a.kind] - rank[b.kind]);
    ordered.forEach((change, n) => {
      if (change.kind === "insert") {
        const row = data.columns.map((c): CellValue => {
          if (c.name in change.values) return toCell(c, change.values[c.name]!);
          if (c.isPrimaryKey && c.hasDefault)
            return {
              kind: "int",
              value: String(Math.max(0, ...rows.map((r) => Number((r[0] as { value: string }).value) || 0)) + 1),
            };
          return { kind: "null" };
        });
        if (find(Object.fromEntries(pk.map((i) => [data.columns[i]!.name, row[i]!]))) >= 0) {
          fail("database", `第 ${n + 1} 条语句执行失败，所有修改都已回滚：Duplicate entry for key 'PRIMARY'`);
        }
        rows = [...rows, row];
        return;
      }
      const index = find(change.key);
      if (index < 0) {
        fail(
          "row_mismatch",
          `第 ${n + 1} 条语句影响了 0 行（应为 1 行）：这一行可能已被别人修改或删除。所有修改都已回滚。`,
        );
      }
      if (change.kind === "delete") rows = rows.filter((_, i) => i !== index);
      else
        rows[index] = data.columns.map((c, i) =>
          c.name in change.values ? toCell(c, change.values[c.name]!) : rows[index]![i]!,
        );
    });
    return rows;
  }

  return {
    /** 清空表,返回删掉的行数。 */
    clear(connectionId: string, table: TableRef): number {
      const data = get(connectionId, table);
      tables.set(keyOf(connectionId, table), { ...data, rows: [] });
      return data.rows.length;
    },
    /** 改名:数据跟着新名字走。 */
    rename(connectionId: string, table: TableRef, name: string): void {
      const data = get(connectionId, table);
      tables.delete(keyOf(connectionId, table));
      tables.set(keyOf(connectionId, { ...table, table: name }), data);
    },
    read(connectionId: string, driver: DriverKind, table: TableRef, request: PageRequest): TablePage {
      const data = get(connectionId, table);
      const filtered = request.filter?.trim() ? data.rows.filter(rowFilter(data.columns, request.filter)) : data.rows;
      const sortIndex = request.sort ? data.columns.findIndex((c) => c.name === request.sort!.column) : -1;
      const sorted =
        sortIndex < 0
          ? filtered
          : [...filtered].sort(
              (a, b) => compareCells(a[sortIndex]!, b[sortIndex]!) * (request.sort!.descending ? -1 : 1),
            );
      const slice = sorted.slice(request.offset, request.offset + request.limit + 1);
      return {
        columns: data.columns,
        rows: slice.slice(0, request.limit),
        hasMore: slice.length > request.limit,
        readOnlyReason: data.columns.some((c) => c.isPrimaryKey)
          ? null
          : "这张表没有主键，无法安全地定位要修改的行，只能查看。",
        sql: selectSql(driver, table, data, request),
      };
    },
    preview(connectionId: string, driver: DriverKind, table: TableRef, changes: RowChange[]): string[] {
      return statements(driver, table, get(connectionId, table), changes);
    },
    apply(
      connectionId: string,
      driver: DriverKind,
      table: TableRef,
      changes: RowChange[],
      expected: string[],
    ): ApplyOutcome {
      const data = get(connectionId, table);
      const planned = statements(driver, table, data, changes);
      if (JSON.stringify(planned) !== JSON.stringify(expected)) {
        fail("invalid_edit", "表结构在预览之后变了，要执行的 SQL 和预览的不一样。请重新预览后再提交。");
      }
      const count = planned.length;
      tables.set(keyOf(connectionId, table), { ...data, rows: applyAll(data, changes) });
      return { statements: count, affectedRows: count };
    },
  };
}
