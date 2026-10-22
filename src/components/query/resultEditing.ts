// 查询结果能不能改、怎么改:SELECT 直接从带主键的表里取的列能改。连接(JOIN)几张表时一次改其中一张,
// 只改格子,不增删行。列的来源由后端按驱动给的元数据标出;自连接这类元数据看不出来的,
// 靠扫一遍语句把住。纯函数,不碰后端。
import type { ResultColumn, ResultSet } from "../../lib/queryTypes";
import type { ColumnInfo, RowChange, TableRef } from "../../lib/types";
import type { SqlDialect } from "./splitStatements";

/** 能改的表(按 FROM 里的先后),和语句是不是连接了几张表。 */
export type SourceCheck = { tables: TableRef[]; joined: boolean } | { reason: string };

export type EditPlan =
  | {
      kind: "editable";
      table: TableRef;
      /** 每个结果列一项:名字是表格里的列名(见 gridNames),其余照表里那一列;别的表的列和表达式列只读。 */
      columns: ColumnInfo[];
      /** 表格里的列名 → 表里的列名。 */
      tableColumn: Readonly<Record<string, string>>;
      /** 能不能新增、删除行:连接了几张表时不能(新行、删掉的行对应不到结果里的一行)。 */
      rowEdits: boolean;
    }
  | { kind: "readOnly"; reason: string };

const NOT_SELECT = "只有 SELECT 查询的结果能改。";
const NOT_PLAIN = "结果来自子查询或 UNION，不能在这里改。";

export const sameTable = (a: TableRef, b: TableRef) =>
  a.database === b.database && (a.schema ?? null) === (b.schema ?? null) && a.table === b.table;

export const tableLabel = (table: TableRef) => (table.schema ? `${table.schema}.${table.table}` : table.table);

type Token = { kind: "word" | "punct"; text: string };

const WORD = /[A-Za-z0-9_$\u0080-￿]/;
const DOLLAR_TAG = /^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/;
// MySQL 以「/*!」开头的注释里是会执行的代码,看不懂就当不是单表。
const EXECUTABLE_COMMENT = "/*!";

/** 去掉注释、字符串;引起来的标识符当成一个词。只用来看语句的骨架,不求精确。 */
function tokens(sql: string, dialect: SqlDialect): Token[] {
  const out: Token[] = [];
  let i = 0;
  const closeAt = (quote: string, from: number, backslash: boolean) => {
    let j = from;
    while (j < sql.length) {
      if (backslash && sql[j] === "\\") j += 2;
      else if (sql[j] === quote && sql[j + 1] === quote) j += 2;
      else if (sql[j] === quote) return j + 1;
      else j += 1;
    }
    return sql.length;
  };
  while (i < sql.length) {
    const ch = sql[i]!;
    const next = sql[i + 1];
    const dollar = dialect === "postgres" && ch === "$" ? DOLLAR_TAG.exec(sql.slice(i))?.[0] : undefined;
    if (/\s/.test(ch)) i += 1;
    else if ((ch === "-" && next === "-") || (ch === "#" && dialect === "mysql")) {
      const end = sql.indexOf("\n", i);
      i = end < 0 ? sql.length : end + 1;
    } else if (ch === "/" && next === "*") {
      if (dialect === "mysql" && sql[i + 2] === "!") out.push({ kind: "punct", text: EXECUTABLE_COMMENT });
      const end = sql.indexOf("*/", i + 2);
      i = end < 0 ? sql.length : end + 2;
    } else if (dollar) {
      // PostgreSQL 的 $tag$ … $tag$ 字符串。
      const end = sql.indexOf(dollar, i + dollar.length);
      out.push({ kind: "word", text: "'" });
      i = end < 0 ? sql.length : end + dollar.length;
    } else if (ch === "'") {
      // MySQL 的字符串里反斜杠转义;PostgreSQL 只有 E'…' 才是。
      const escaped =
        dialect === "mysql" ||
        (dialect === "postgres" && /[Ee]/.test(sql[i - 1] ?? "") && !WORD.test(sql[i - 2] ?? ""));
      out.push({ kind: "word", text: "'" });
      i = closeAt("'", i + 1, escaped);
    } else if (ch === '"' || ch === "`" || (ch === "[" && dialect === "sqlite")) {
      const close = ch === "[" ? "]" : ch;
      const end = closeAt(close, i + 1, false);
      out.push({ kind: "word", text: sql.slice(i, end) });
      i = end;
    } else if (WORD.test(ch)) {
      let end = i + 1;
      while (end < sql.length && WORD.test(sql[end]!)) end += 1;
      out.push({ kind: "word", text: sql.slice(i, end) });
      i = end;
    } else {
      out.push({ kind: "punct", text: ch });
      i += 1;
    }
  }
  return out;
}

/** 名字的一段:去掉引号;不带引号的数据库按不分大小写处理。 */
interface NamePart {
  text: string;
  quoted: boolean;
}

function namePart(token: Token): NamePart {
  const quote = token.text[0];
  if (quote === '"' || quote === "`" || quote === "[") {
    const close = quote === "[" ? "]" : quote;
    return { text: token.text.slice(1, -1).replaceAll(close + close, close), quoted: true };
  }
  return { text: token.text, quoted: false };
}

/** FROM 后面可以接的子句;碰到它们表名部分就结束了。 */
const CLAUSES = new Set([
  "WHERE",
  "GROUP",
  "HAVING",
  "ORDER",
  "LIMIT",
  "OFFSET",
  "FETCH",
  "FOR",
  "WINDOW",
  "LOCK",
  "QUALIFY",
]);
const SET_OPERATORS = new Set(["UNION", "INTERSECT", "EXCEPT", "MINUS"]);

/** FROM 里的一张表:限定名按段拆开,没写别名时 alias 为 null。 */
export interface FromTable {
  name: NamePart[];
  alias: NamePart | null;
}

export type StatementShape =
  | { kind: "notSelect" }
  /** 子查询当表、UNION 这类集合运算、看不懂的 FROM。 */
  | { kind: "multiple" }
  /**
   * `items`:选的每一项是不是直接写的列(`[表.]列 [[AS] 别名]`),和结果列一一对应;
   * 有 `*` 时对应不上,为 "star"。`subquery`:选的列里有没有子查询。
   */
  | { kind: "tables"; tables: FromTable[]; items: boolean[] | "star"; subquery: boolean };

/** 连接两张表的关键字:`[NATURAL] [INNER | LEFT | RIGHT | FULL | CROSS] [OUTER] JOIN`,MySQL 还有 STRAIGHT_JOIN。 */
const JOIN_WORDS = new Set(["JOIN", "STRAIGHT_JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "OUTER", "NATURAL"]);
const JOINS = new Set(["JOIN", "STRAIGHT_JOIN"]);
const NOT_ALIAS = new Set([...JOIN_WORDS, "ON", "USING"]);

const upper = (token: Token | undefined) => (token?.kind === "word" ? token.text.toUpperCase() : "");
const isName = (token: Token | undefined) => token?.kind === "word" && !NOT_ALIAS.has(upper(token));

/**
 * FROM 后面的表:`表 [AS] [别名]`,之间用逗号或 JOIN 连接,JOIN 后面可以跟 ON 条件或 USING (…)。
 * 表的位置上出现括号(子查询、LATERAL)或别的写法就认不出,返回 null。
 */
function fromTables(part: readonly Token[]): FromTable[] | null {
  const tables: FromTable[] = [];
  let at = 0;
  for (;;) {
    if (!isName(part[at]) || upper(part[at]) === "LATERAL") return null;
    const name = [namePart(part[at]!)];
    at += 1;
    while (part[at]?.text === "." && isName(part[at + 1])) {
      name.push(namePart(part[at + 1]!));
      at += 2;
    }
    if (upper(part[at]) === "AS") at += 1;
    const alias = isName(part[at]) ? namePart(part[at]!) : null;
    if (alias) at += 1;
    tables.push({ name, alias });
    // 连接条件一直到下一个不在括号里的逗号或 JOIN。
    if (upper(part[at]) === "ON" || upper(part[at]) === "USING") {
      let depth = 0;
      at += 1;
      while (at < part.length) {
        const token = part[at]!;
        if (token.text === "(") depth += 1;
        else if (token.text === ")") depth -= 1;
        else if (depth === 0 && (token.text === "," || JOIN_WORDS.has(upper(token)))) break;
        at += 1;
      }
    }
    if (at >= part.length) return tables;
    if (part[at]!.text === ",") {
      at += 1;
      continue;
    }
    let joined = false;
    while (JOIN_WORDS.has(upper(part[at]))) {
      joined ||= JOINS.has(upper(part[at]));
      at += 1;
    }
    if (!joined) return null;
  }
}

/** 选的列前面可以有的修饰词:`DISTINCT`、`ALL`,PostgreSQL 的 `DISTINCT ON (…)`。 */
const MODIFIERS = new Set(["DISTINCT", "ALL"]);

/** 拆开 SELECT 和 FROM 之间的列表:每一项是不是直接写的列,有没有 `*`、子查询。 */
function selectItems(list: readonly Token[]): { items: boolean[] | "star"; subquery: boolean } {
  let at = 0;
  while (MODIFIERS.has(upper(list[at]))) at += 1;
  if (upper(list[at]) === "ON" && list[at + 1]?.text === "(") {
    let depth = 0;
    for (at += 1; at < list.length; at += 1) {
      if (list[at]!.text === "(") depth += 1;
      else if (list[at]!.text === ")" && --depth === 0) break;
    }
    at += 1;
  }
  const items: Token[][] = [[]];
  let depth = 0;
  let subquery = false;
  for (const token of list.slice(at)) {
    if (token.text === "(") depth += 1;
    else if (token.text === ")") depth -= 1;
    else if (upper(token) === "SELECT") subquery = true;
    if (depth === 0 && token.text === ",") items.push([]);
    else items.at(-1)!.push(token);
  }
  if (items.some((item) => item.at(-1)?.text === "*")) return { items: "star", subquery };
  return { items: items.map(plainColumn), subquery };
}

/** `[限定.]列 [[AS] 别名]`:没有括号、运算符、函数。 */
function plainColumn(item: readonly Token[]): boolean {
  let at = 0;
  if (item[at]?.kind !== "word") return false;
  at += 1;
  while (item[at]?.text === "." && item[at + 1]?.kind === "word") at += 2;
  if (upper(item[at]) === "AS") at += 1;
  if (item[at]?.kind === "word") at += 1;
  return at === item.length;
}

/**
 * 语句是不是「SELECT … FROM 表 [JOIN 表 …] [WHERE …]…」:FROM 里没有子查询当表,
 * 整条语句没有 UNION 这类集合运算。返回 FROM 里写的每一张表。
 */
export function statementShape(sql: string, dialect: SqlDialect): StatementShape {
  const list = tokens(sql, dialect);
  if (list[0]?.text.toUpperCase() !== "SELECT") return { kind: "notSelect" };
  if (list.some((t) => t.text === EXECUTABLE_COMMENT)) return { kind: "multiple" };
  let depth = 0;
  let from = -1;
  for (const [index, token] of list.entries()) {
    if (token.text === "(") depth += 1;
    else if (token.text === ")") depth -= 1;
    else if (depth === 0 && token.kind === "word") {
      const word = token.text.toUpperCase();
      if (SET_OPERATORS.has(word)) return { kind: "multiple" };
      if (word === "FROM" && from < 0) from = index;
    }
  }
  if (from < 0) return { kind: "multiple" };
  const { items, subquery } = selectItems(list.slice(1, from));
  // 表的部分到下一个不在括号里的子句或语句结尾为止(ON 条件里可能有带 WHERE 的子查询)。
  const part: Token[] = [];
  depth = 0;
  for (const token of list.slice(from + 1)) {
    if (token.text === "(") depth += 1;
    else if (token.text === ")") depth -= 1;
    else if (depth === 0 && (token.text === ";" || (token.kind === "word" && CLAUSES.has(upper(token))))) break;
    part.push(token);
  }
  const tables = fromTables(part);
  return tables ? { kind: "tables", tables, items, subquery } : { kind: "multiple" };
}

const samePart = (written: NamePart | undefined, actual: string | null | undefined) =>
  written === undefined ||
  (actual != null && (written.quoted ? written.text === actual : written.text.toLowerCase() === actual.toLowerCase()));

/**
 * FROM 里写的就是列的来源那张表。视图查出来的列,SQLite 报的是视图底下的表:
 * 视图里可能是自连接或 UNION,按来源改会改错行,所以名字对不上就不让改。
 */
function writtenIs(name: NamePart[], table: TableRef, dialect: SqlDialect): boolean {
  const [qualifier, last] = name.length > 1 ? [name.at(-2), name.at(-1)] : [undefined, name[0]];
  return samePart(last, table.table) && samePart(qualifier, dialect === "postgres" ? table.schema : table.database);
}

type Sourced = ResultColumn & { source: NonNullable<ResultColumn["source"]> };

/** 一张表为什么不能在这里改;能改时为 null。 */
function tableProblem(table: TableRef, tables: readonly FromTable[], sourced: readonly Sourced[], dialect: SqlDialect) {
  const written = tables.filter((t) => writtenIs(t.name, table, dialect)).length;
  if (written === 0) {
    return `结果的列来自 ${tableLabel(table)}，不是 FROM 里写的那张（比如视图），不能在这里改。`;
  }
  if (written > 1) {
    return `${tableLabel(table)} 在 FROM 里出现了不止一次，分不清结果里的列是哪一次的，不能在这里改。`;
  }
  const columns = sourced.filter((c) => sameTable(c.source.table, table)).map((c) => c.source.column);
  if (new Set(columns).size !== columns.length) return "同一列在结果里出现了不止一次，不能在这里改。";
  return null;
}

/**
 * 只信直接写出来的列的来源:SQLite 把子查询算出来的列也报成某张表的列(`(SELECT name FROM emp …) AS boss`
 * 报成 emp.name),按它改会改错行。选的列和结果列一一对应时,不是直接写的列一律当表达式。
 */
export function trustedSources(sql: string, set: ResultSet, dialect: SqlDialect): ResultSet {
  const shape = statementShape(sql, dialect);
  if (shape.kind !== "tables" || shape.items === "star" || shape.items.length !== set.columns.length) return set;
  const items = shape.items;
  if (items.every(Boolean)) return set;
  return { ...set, columns: set.columns.map((c, i) => (items[i] ? c : { ...c, source: null })) };
}

const nameText = (name: readonly NamePart[]) => name.map((part) => part.text).join(".");

/** 不用问后端就能判断的:语句的形状、列来自哪几张表、哪几张能改。 */
export function sourceTables(sql: string, set: ResultSet, dialect: SqlDialect): SourceCheck {
  const shape = statementShape(sql, dialect);
  if (shape.kind === "notSelect") return { reason: NOT_SELECT };
  if (shape.kind === "multiple") return { reason: NOT_PLAIN };
  if (set.readOnly) return { reason: set.readOnly };
  const columns = trustedSources(sql, set, dialect).columns;
  const sourced = columns.filter((c): c is Sourced => Boolean(c.source));
  if (sourced.length === 0) return { reason: "结果里没有直接取自表的列，不能在这里改。" };
  const distinct = sourced
    .map((c) => c.source.table)
    .filter((table, index, all) => all.findIndex((t) => sameTable(t, table)) === index);
  if (dialect === "sqlite") {
    // SQLite 报的视图、子查询列的来源是底下的表,和 FROM 里的另一张表撞上就会改错行。
    if (shape.subquery && (shape.items === "star" || shape.items.length !== set.columns.length)) {
      return { reason: "选的列里有子查询又有 *，分不清哪一列是子查询算出来的，不能在这里改。" };
    }
    const unmatched = shape.tables.find((t) => !distinct.some((table) => writtenIs(t.name, table, dialect)));
    if (unmatched) {
      return {
        reason: `FROM 里的 ${nameText(unmatched.name)} 在结果里没有列：它可能是视图（SQLite 把视图的列报成底下的表），分不清，不能在这里改。`,
      };
    }
  }
  const problems = distinct.map((table) => ({ table, problem: tableProblem(table, shape.tables, sourced, dialect) }));
  const editable = problems.filter((p) => p.problem === null).map((p) => p.table);
  if (editable.length === 0) return { reason: problems[0]!.problem! };
  // 按 FROM 里的先后排:第一张通常是「主表」。
  const position = (table: TableRef) => shape.tables.findIndex((t) => writtenIs(t.name, table, dialect));
  return { tables: editable.sort((a, b) => position(a) - position(b)), joined: shape.tables.length > 1 };
}

/**
 * 表格按列名认列。结果里同名的列(连接两张表时的 id)加上表名区分,还重的(表达式、同一张表)再加序号;
 * 不重名的列照结果里的名字。
 */
export function gridNames(columns: readonly ResultColumn[]): string[] {
  const count = new Map<string, number>();
  for (const column of columns) count.set(column.name, (count.get(column.name) ?? 0) + 1);
  const used = new Set<string>();
  return columns.map((column, index) => {
    let name = column.name;
    if ((count.get(name) ?? 0) > 1 && column.source) name = `${name}（${column.source.table.table}）`;
    if (used.has(name)) name = `${name} #${index + 1}`;
    used.add(name);
    return name;
  });
}

/** 别的表的列、表达式列:能看不能改,新增行里也不填。 */
function lockedColumn(column: ResultColumn, name: string): ColumnInfo {
  return {
    name,
    dataType: column.dataType,
    isPrimaryKey: false,
    nullable: true,
    hasDefault: false,
    isGenerated: true,
    valueType: { kind: "text" },
  };
}

/** 读到表的列信息之后:主键要齐全,这张表的每个结果列对上表里的列。 */
export function editPlan(
  set: ResultSet,
  table: TableRef,
  tableColumns: readonly ColumnInfo[],
  joined = false,
): EditPlan {
  const keys = tableColumns.filter((c) => c.isPrimaryKey);
  if (keys.length === 0) return { kind: "readOnly", reason: `${tableLabel(table)} 没有主键，不能在这里改。` };
  const own = (column: ResultColumn) => column.source !== null && sameTable(column.source.table, table);
  const shown = new Set(set.columns.flatMap((c) => (own(c) ? [c.source!.column] : [])));
  const missing = keys.filter((k) => !shown.has(k.name)).map((k) => k.name);
  if (missing.length > 0) {
    const where = joined ? ` ${tableLabel(table)} 的` : "";
    return { kind: "readOnly", reason: `结果里没有${where}主键列 ${missing.join("、")}，不能在这里改。` };
  }
  const names = gridNames(set.columns);
  const tableColumn: Record<string, string> = {};
  const columns = set.columns.map((column, index) => {
    const name = names[index]!;
    const info = own(column) ? tableColumns.find((c) => c.name === column.source!.column) : undefined;
    if (!info) return lockedColumn(column, name);
    tableColumn[name] = info.name;
    return { ...info, name };
  });
  return { kind: "editable", table, columns, tableColumn, rowEdits: !joined };
}

/**
 * 连接了几张表的结果里改的是一张表的行,提交前查两种对不上的情况:外连接没匹配上的行
 * (这张表的主键是 NULL),和同一行在结果里出现了几次、改成了不同的值。没问题时为 null。
 */
export function joinedChangeProblem(changes: readonly RowChange[], table: TableRef): string | null {
  const seen = new Map<string, Record<string, unknown>>();
  const updates = changes.filter((change) => change.kind === "update");
  const keyOf = (change: (typeof updates)[number]) => JSON.stringify(change.key);
  for (const change of updates) {
    if (Object.values(change.key).some((value) => value.kind === "null")) {
      return `有的行在 ${tableLabel(table)} 里没有对应的行（外连接没匹配上），不能改。`;
    }
    const key = keyOf(change);
    // 改了主键,同一行的其他修改按旧主键就找不到它了。
    const keyEdited = Object.keys(change.values).some((column) => column in change.key);
    if (keyEdited && updates.some((other) => other !== change && keyOf(other) === key)) {
      return `${tableLabel(table)} 里的同一行在结果里出现了几次，又改了它的主键，不能一起提交。`;
    }
    const before = seen.get(key) ?? {};
    for (const [column, value] of Object.entries(change.values)) {
      if (column in before && JSON.stringify(before[column]) !== JSON.stringify(value)) {
        return `${tableLabel(table)} 里的同一行在结果里出现了几次，「${column}」改成了不同的值。`;
      }
    }
    seen.set(key, { ...before, ...change.values });
  }
  return null;
}

const renamed = <T>(values: Readonly<Record<string, T>>, names: Readonly<Record<string, string>>) =>
  Object.fromEntries(Object.entries(values).map(([name, value]) => [names[name] ?? name, value]));

/** 表格按结果里的列名记修改;交给后端前换成表里的列名。 */
export function toTableChanges(changes: readonly RowChange[], names: Readonly<Record<string, string>>): RowChange[] {
  return changes.map((change) =>
    change.kind === "insert"
      ? { kind: "insert", values: renamed(change.values, names) }
      : change.kind === "update"
        ? { kind: "update", key: renamed(change.key, names), values: renamed(change.values, names) }
        : { kind: "delete", key: renamed(change.key, names) },
  );
}
