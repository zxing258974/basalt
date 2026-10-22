// 美化的保险:把美化前后的 SQL 都切成词,一个词一个词地对。只许改空白(和不分大小写时的大小写),
// 词变了就不换;格式化程序在几处会改坏意思的地方加了空格,照原样粘回去。
import type { SqlDialect } from "./splitStatements";

export type TokenKind = "word" | "string" | "quoted" | "comment" | "lineComment" | "punct";

export interface SqlToken {
  kind: TokenKind;
  text: string;
  start: number;
  /** 前面的空白(和上一个词之间)。 */
  sep: string;
}

const WORD = /[\p{L}\p{N}_$]/u;
const SPACE = /\s/;
/** 运算符里的字:挨着的两个拆开可能就成了两个运算符(`->>`、`::`、`@>`、自定义的 `===`)。 */
const OPERATOR = /[+\-*/<>=~!@#%^&|`?:]/;
/** 字符串前面的前缀:X'…'、B'…'、N'…'、E'…',MySQL 的字符集 _utf8mb4'…'。 */
const STRING_PREFIX = /^(?:[xXbBnNeE]|_\w+)$/;
/** 挨在名字前面的:MySQL 的 @变量、各家的参数(:name、?1)。 */
const SIGIL = /[@:?$]/;

function closeQuote(sql: string, at: number, quote: string, backslash: boolean): number {
  let end = at + 1;
  while (end < sql.length) {
    const ch = sql[end]!;
    if (backslash && ch === "\\") end += 2;
    else if (ch === quote && sql[end + 1] === quote) end += 2;
    else if (ch === quote) return end + 1;
    else end += 1;
  }
  return sql.length;
}

/** PostgreSQL 的块注释可以嵌套。 */
function closeBlockComment(sql: string, at: number, nested: boolean): number {
  let depth = 0;
  let end = at;
  while (end < sql.length) {
    if (sql.startsWith("/*", end)) {
      depth = nested ? depth + 1 : 1;
      end += 2;
    } else if (sql.startsWith("*/", end)) {
      depth -= 1;
      end += 2;
      if (depth === 0) return end;
    } else end += 1;
  }
  return sql.length;
}

/** 切词:字符串、带引号的名字、注释各算一个词,别的字母数字连成词,其余一个字一个词。 */
export function sqlTokens(sql: string, dialect: SqlDialect): SqlToken[] {
  const tokens: SqlToken[] = [];
  let at = 0;
  let sep = "";
  const push = (kind: TokenKind, end: number) => {
    tokens.push({ kind, text: sql.slice(at, end), start: at, sep });
    sep = "";
    at = end;
  };
  while (at < sql.length) {
    const ch = sql[at]!;
    const next = sql[at + 1] ?? "";
    if (SPACE.test(ch)) {
      sep += ch;
      at += 1;
      continue;
    }
    // MySQL 的 -- 后面要跟空白才是注释(--1 是负负一)。
    const third = sql[at + 2];
    const dashComment = ch === "-" && next === "-" && (dialect !== "mysql" || third === undefined || /\s/.test(third));
    if (dashComment || (ch === "#" && dialect === "mysql")) {
      const end = sql.indexOf("\n", at);
      push("lineComment", end < 0 ? sql.length : end);
    } else if (ch === "/" && next === "*") {
      push("comment", closeBlockComment(sql, at, dialect === "postgres"));
    } else if (ch === "'") {
      // MySQL 的字符串里反斜杠转义;PostgreSQL 只有 E'…' 才是。
      const prefix = tokens.at(-1);
      const escaped =
        dialect === "mysql" ||
        (dialect === "postgres" && sep === "" && prefix?.kind === "word" && /^[Ee]$/.test(prefix.text));
      push("string", closeQuote(sql, at, "'", escaped));
    } else if (ch === '"') {
      push(dialect === "mysql" ? "string" : "quoted", closeQuote(sql, at, '"', dialect === "mysql"));
    } else if (ch === "`" && dialect !== "postgres") {
      push("quoted", closeQuote(sql, at, "`", false));
    } else if (ch === "[" && dialect === "sqlite") {
      const end = sql.indexOf("]", at);
      push("quoted", end < 0 ? sql.length : end + 1);
    } else if (ch === "$" && dialect === "postgres" && /^\$(?:[A-Za-z_]\w*)?\$/.test(sql.slice(at))) {
      const tag = /^\$(?:[A-Za-z_]\w*)?\$/.exec(sql.slice(at))![0];
      const end = sql.indexOf(tag, at + tag.length);
      push("string", end < 0 ? sql.length : end + tag.length);
    } else if (WORD.test(ch)) {
      let end = at + 1;
      while (end < sql.length && WORD.test(sql[end]!)) end += 1;
      push("word", end);
    } else {
      push("punct", at + 1);
    }
  }
  return tokens;
}

/** 两个原来挨着的词,中间加了空格会不会变意思。 */
function mustStayGlued(before: SqlToken, after: SqlToken): boolean {
  if (before.kind === "word" && after.text === "(") return true; // 函数名和括号:MySQL 的内置函数不许有空格
  if (before.kind === "punct" && after.kind === "punct") return OPERATOR.test(before.text) && OPERATOR.test(after.text);
  if (after.kind === "string" && STRING_PREFIX.test(before.text)) return true;
  if (before.kind === "punct" && SIGIL.test(before.text) && after.kind === "word") return true;
  return before.text === "." || after.text === ".";
}

const sameText = (a: SqlToken, b: SqlToken, ignoreCase: boolean) =>
  a.text === b.text || (ignoreCase && a.kind === "word" && a.text.toLowerCase() === b.text.toLowerCase());

/**
 * 美化后的 SQL 和原来的词一样(`ignoreCase`:不加引号的名字和关键字不分大小写)时,修好几处空白再返回;
 * 词对不上返回 null(比如代码被挪到了行注释后面,就成了注释的一部分)。修的地方:原来挨着、拆开会变意思的
 * 又粘回去;两个挨着的字符串之间的空白照原样(PostgreSQL 隔行写的两段字符串会连起来,换成空格就不行了)。
 */
export function guardFormatted(
  original: string,
  formatted: string,
  dialect: SqlDialect,
  ignoreCase: boolean,
): { text: string; tokens: SqlToken[] } | null {
  const before = sqlTokens(original, dialect);
  const after = sqlTokens(formatted, dialect);
  if (before.length !== after.length) return null;
  const repaired: SqlToken[] = [];
  let position = 0;
  for (const [i, token] of after.entries()) {
    const old = before[i]!;
    if (token.kind !== old.kind || !sameText(token, old, ignoreCase)) return null;
    let sep = token.sep;
    if (i > 0) {
      const previous = before[i - 1]!;
      if (old.sep === "" && mustStayGlued(previous, old)) sep = "";
      if (previous.kind === "string" && old.kind === "string") sep = old.sep;
    }
    repaired.push({ ...token, sep, start: position + sep.length });
    position += sep.length + token.text.length;
  }
  const trailing = formatted.slice(formatted.trimEnd().length);
  return { text: repaired.map((t) => t.sep + t.text).join("") + trailing, tokens: repaired };
}

/** `at` 落在字符串、带引号的名字、注释中间(不在它的开头或结尾)。 */
export function insideLiteral(tokens: readonly SqlToken[], at: number): boolean {
  return tokens.some((t) => t.kind !== "word" && t.kind !== "punct" && t.start < at && at < t.start + t.text.length);
}
