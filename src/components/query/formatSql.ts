// SQL 美化:按连接的数据库选方言,两格缩进,语句之间空一行。sql-formatter 不小,只在第一次美化时才加载。
// 美化完逐词核对(见 formatGuard):只许改空白,词对不上就不换。
import type { DriverKind } from "../../lib/types";
import { guardFormatted, insideLiteral, sqlTokens, type SqlToken } from "./formatGuard";
import { editorDialect } from "./SqlEditor";

export type FormatResult =
  /** 把 [from, to) 换成 text;cursor 是没选中文字时光标该去的地方(同一个词上)。 */
  | { kind: "edit"; from: number; to: number; text: string; cursor: number | null }
  /** 没有 SQL:编辑器是空的,或者选中的只有空白。 */
  | { kind: "empty"; selection: boolean }
  /** 已经是美化后的样子。 */
  | { kind: "unchanged" };

/** sql-formatter 会把 `DELIMITER //` 拆成 `/ /`,改坏整段脚本。Basalt 执行存储过程本来也不用它。 */
const DELIMITER = /^\s*DELIMITER\s/im;
const PARSE_ERROR = /at line (\d+) column (\d+)/;

/** 读不懂的地方:行、列从 1 数,相对交给格式化程序的那段。 */
class ParseError extends Error {
  constructor(
    readonly line: number,
    readonly column: number,
    cause: unknown,
  ) {
    super(`第 ${line} 行第 ${column} 列附近读不懂`, { cause });
  }
}

/**
 * MySQL / MariaDB 在 Linux 上表名、别名分大小写,`AS Text` 改成 `AS TEXT` 列名也变了:关键字照原样。
 * PostgreSQL、SQLite 不加引号的名字本来就不分大小写,关键字、类型名大写。
 */
const keepsCase = (driver: DriverKind | null) => driver !== "postgres" && driver !== "sqlite";

/** 美化一段 SQL,逐词核对过。读不懂、核对不上时以 Error reject,原文不动。 */
async function formatChecked(sql: string, driver: DriverKind | null): Promise<{ text: string; tokens: SqlToken[] }> {
  const formatter = await import("sql-formatter");
  const dialects = {
    mysql: formatter.mysql,
    mariadb: formatter.mariadb,
    postgres: formatter.postgresql,
    sqlite: formatter.sqlite,
  } satisfies Record<DriverKind, unknown>;
  const letterCase = keepsCase(driver) ? "preserve" : "upper";
  let formatted: string;
  try {
    formatted = formatter.formatDialect(sql, {
      dialect: dialects[driver ?? "mysql"],
      keywordCase: letterCase,
      dataTypeCase: letterCase,
      functionCase: "preserve",
      tabWidth: 2,
      linesBetweenQueries: 1,
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const at = PARSE_ERROR.exec(message);
    throw at
      ? new ParseError(Number(at[1]), Number(at[2]), error)
      : new Error(message.split("\n")[0] ?? message, { cause: error });
  }
  const checked = guardFormatted(sql, formatted, editorDialect(driver), !keepsCase(driver));
  if (!checked) throw new Error("美化后的语句和原来对不上（可能是少见的写法），没有改");
  return checked;
}

export async function formatSql(sql: string, driver: DriverKind | null): Promise<string> {
  return (await formatChecked(sql, driver)).text;
}

/** 文档里 `offset` 处是第几行(从 0 数)、这一行从哪里开始。 */
function lineOf(text: string, offset: number): { line: number; lineStart: number } {
  const before = text.slice(0, offset);
  return { line: before.split("\n").length - 1, lineStart: before.lastIndexOf("\n") + 1 };
}

/** 美化前光标在第几个词的第几个字,美化后放到同一个词的同一个字上。 */
function mapCursor(
  body: string,
  rel: number,
  formatted: { text: string; tokens: SqlToken[] },
  driver: DriverKind | null,
): number {
  if (rel >= body.length) return formatted.text.length + (rel - body.length);
  const tokens = sqlTokens(body, editorDialect(driver));
  const index = tokens.findIndex((t) => rel < t.start + t.text.length);
  const target = formatted.tokens[index];
  if (index < 0 || !target) return formatted.text.length;
  return target.start + Math.max(0, rel - tokens[index]!.start);
}

/**
 * 有选中的文字就只美化选中的部分,否则美化整个编辑器。前后的空白照留,
 * 免得选中一条语句美化以后和下一条粘在一起。
 */
export async function formatEdit(
  text: string,
  selection: { from: number; to: number },
  driver: DriverKind | null,
): Promise<FormatResult> {
  // 选中的部分里没有也不行:脚本别处的 DELIMITER 改了分隔符,选中的 `END //` 美化后也会被拆开。
  if (DELIMITER.test(text)) {
    throw new Error("里面有 DELIMITER，美化会把分隔符拆开，先去掉它（Basalt 执行存储过程不需要 DELIMITER）");
  }
  const selected = selection.from !== selection.to;
  const from = selected ? Math.min(selection.from, selection.to) : 0;
  const to = selected ? Math.max(selection.from, selection.to) : text.length;
  if (selected) {
    const tokens = sqlTokens(text, editorDialect(driver));
    if (insideLiteral(tokens, from) || insideLiteral(tokens, to)) {
      throw new Error("选中的范围从字符串或注释的中间开始或结束，请选完整的语句");
    }
  }
  const picked = text.slice(from, to);
  const body = picked.trim();
  if (!body) return { kind: "empty", selection: selected };
  const lead = picked.slice(0, picked.indexOf(body));
  const trail = picked.slice(picked.indexOf(body) + body.length);
  const start = from + lead.length;
  let formatted: { text: string; tokens: SqlToken[] };
  try {
    formatted = await formatChecked(body, driver);
  } catch (error) {
    if (!(error instanceof ParseError)) throw error;
    // 换成在整个编辑器里的行列。
    const { line, lineStart } = lineOf(text, start);
    const column = error.line === 1 ? start - lineStart + error.column : error.column;
    throw new Error(`第 ${line + error.line} 行第 ${column} 列附近读不懂`, { cause: error });
  }
  const replaced = `${lead}${formatted.text}${trail}`;
  if (replaced === picked) return { kind: "unchanged" };
  const cursor = selected
    ? null
    : selection.to < start
      ? selection.to
      : start + mapCursor(body, selection.to - start, formatted, driver);
  return { kind: "edit", from, to, text: replaced, cursor };
}
