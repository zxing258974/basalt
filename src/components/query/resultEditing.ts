// 查询结果能不能改、怎么改:只有从一张带主键的表里直接取列的 SELECT 能改。
// 列的来源由后端按驱动给的元数据标出;自连接这类元数据看不出来的,靠扫一遍语句把住。纯函数,不碰后端。
import type { ResultColumn, ResultSet } from "../../lib/queryTypes";
import type { ColumnInfo, RowChange, TableRef } from "../../lib/types";
import type { SqlDialect } from "./splitStatements";

export type SourceCheck = { table: TableRef } | { reason: string };

export type EditPlan =
  | {
      kind: "editable";
      table: TableRef;
      /** 每个结果列一项:名字是结果里的名字,其余照表里那一列;表达式列只读。 */
      columns: ColumnInfo[];
      /** 结果列名 → 表里的列名。 */
      tableColumn: Readonly<Record<string, string>>;
    }
  | { kind: "readOnly"; reason: string };

const NOT_SELECT = "只有 SELECT 查询的结果能改。";
const NOT_SINGLE = "结果来自多张表、子查询或 UNION，不能在这里改。";

const sameTable = (a: TableRef, b: TableRef) =>
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

export type StatementShape =
  | { kind: "notSelect" }
  | { kind: "multiple" }
  /** `name` 是 FROM 后面写的表名,限定名按段拆开。 */
  | { kind: "single"; name: NamePart[] };

/**
 * 语句是不是「SELECT … FROM 一张表 [别名] [WHERE …]…」:FROM 里没有逗号、JOIN、子查询,
 * 整条语句没有 UNION 这类集合运算。同一张表出现两次(自连接)时列的来源一样,却不是同一行,只能在这里拦住。
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
  // 表名部分到下一个子句或语句结尾为止,只能是:限定名(a.b) [AS] [别名]。
  const part: Token[] = [];
  for (const token of list.slice(from + 1)) {
    if (token.text === ";" || (token.kind === "word" && CLAUSES.has(token.text.toUpperCase()))) break;
    part.push(token);
  }
  const isName = (token: Token | undefined) => token?.kind === "word";
  if (!isName(part[0])) return { kind: "multiple" };
  const name = [namePart(part[0]!)];
  let at = 1;
  while (part[at]?.text === "." && isName(part[at + 1])) {
    name.push(namePart(part[at + 1]!));
    at += 2;
  }
  if (part[at]?.text.toUpperCase() === "AS") at += 1;
  if (isName(part[at])) at += 1;
  return at === part.length ? { kind: "single", name } : { kind: "multiple" };
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

/** 不用问后端就能判断的:语句的形状、列来自哪张表。 */
export function sourceTable(sql: string, set: ResultSet, dialect: SqlDialect): SourceCheck {
  const shape = statementShape(sql, dialect);
  if (shape.kind === "notSelect") return { reason: NOT_SELECT };
  if (shape.kind === "multiple") return { reason: NOT_SINGLE };
  if (set.readOnly) return { reason: set.readOnly };
  const sourced = set.columns.filter((c): c is ResultColumn & { source: NonNullable<ResultColumn["source"]> } =>
    Boolean(c.source),
  );
  const first = sourced[0];
  if (!first) return { reason: "结果里没有直接取自表的列，不能在这里改。" };
  if (sourced.some((c) => !sameTable(c.source.table, first.source.table))) return { reason: NOT_SINGLE };
  if (!writtenIs(shape.name, first.source.table, dialect)) {
    return {
      reason: `结果的列来自 ${tableLabel(first.source.table)}，不是 FROM 里写的那张（比如视图），不能在这里改。`,
    };
  }
  const names = sourced.map((c) => c.name);
  if (new Set(names).size !== names.length) return { reason: "结果里有同名的列，不能在这里改。" };
  const columns = sourced.map((c) => c.source.column);
  if (new Set(columns).size !== columns.length) return { reason: "同一列在结果里出现了不止一次，不能在这里改。" };
  return { table: first.source.table };
}

/** 表达式列:能看不能改,新增行里也不填。 */
function expressionColumn(column: ResultColumn): ColumnInfo {
  return {
    name: column.name,
    dataType: column.dataType,
    isPrimaryKey: false,
    nullable: true,
    hasDefault: false,
    isGenerated: true,
    valueType: { kind: "text" },
  };
}

/** 读到表的列信息之后:主键要齐全,每个结果列对上表里的列。 */
export function editPlan(set: ResultSet, table: TableRef, tableColumns: readonly ColumnInfo[]): EditPlan {
  const keys = tableColumns.filter((c) => c.isPrimaryKey);
  if (keys.length === 0) return { kind: "readOnly", reason: `${tableLabel(table)} 没有主键，不能在这里改。` };
  const shown = new Set(set.columns.flatMap((c) => (c.source ? [c.source.column] : [])));
  const missing = keys.filter((k) => !shown.has(k.name)).map((k) => k.name);
  if (missing.length > 0) {
    return { kind: "readOnly", reason: `结果里没有主键列 ${missing.join("、")}，不能在这里改。` };
  }
  const tableColumn: Record<string, string> = {};
  const columns = set.columns.map((column) => {
    const info = column.source && tableColumns.find((c) => c.name === column.source!.column);
    if (!info) return expressionColumn(column);
    tableColumn[column.name] = info.name;
    return { ...info, name: column.name };
  });
  return { kind: "editable", table, columns, tableColumn };
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
