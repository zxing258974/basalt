// SQL 补全的纯逻辑:光标前在补什么(限定名还是裸词)、语句里引用了哪些表和别名。
// 只为补全服务,不求完整解析 SQL,认不出就少给几个候选。

export interface CompletionTarget {
  /** `x.` 里的 x(去掉引号);没有限定时为 null。 */
  qualifier: string | null;
  /** 光标前正在输入的词,可能为空。 */
  word: string;
}

const WORD_CHAR = /[\w$\u0080-\uffff]/;

function unquote(name: string): string {
  const first = name[0];
  if ((first === "`" || first === '"') && name.endsWith(first) && name.length >= 2) {
    return name.slice(1, -1).replaceAll(first + first, first);
  }
  return name;
}

/** 从光标前的文本末尾读出 `限定.词` 或 `词`。 */
export function completionTarget(before: string): CompletionTarget {
  let start = before.length;
  while (start > 0 && WORD_CHAR.test(before[start - 1]!)) start -= 1;
  const word = before.slice(start);
  if (before[start - 1] !== ".") return { qualifier: null, word };
  const head = before.slice(0, start - 1);
  const quoted = /(`(?:[^`]|``)*`|"(?:[^"]|"")*")$/.exec(head);
  if (quoted) return { qualifier: unquote(quoted[1]!), word };
  const bare = /([\w$\u0080-\uffff]+)$/.exec(head);
  return { qualifier: bare ? bare[1]! : null, word };
}

export interface TableReference {
  /** 库名(MySQL、SQLite)或 schema 名(PostgreSQL);没写时为 null。 */
  namespace: string | null;
  table: string;
  alias: string | null;
}

const NAME = String.raw`(?:\x60(?:[^\x60]|\x60\x60)*\x60|"(?:[^"]|"")*"|[\w$\u0080-\uffff]+)`;
const QUALIFIED = String.raw`(${NAME})(?:\s*\.\s*(${NAME}))?`;
/** 别名后面不能是这些关键字(它们是语句的下一部分)。 */
const NOT_ALIAS = new Set(
  "where on using join left right inner outer full cross natural group order having limit union set values select and or as window for lock offset fetch returning into straight_join".split(
    " ",
  ),
);

/** 去掉注释和字符串内容,免得里面的 FROM 被当成表引用。 */
function stripLiterals(sql: string): string {
  return sql
    .replace(/--[^\n]*|#[^\n]*/g, " ")
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/'(?:[^'\\]|\\.|'')*'/g, "''");
}

function reference(first: string, second: string | undefined, alias: string | undefined): TableReference {
  const table = unquote(second ?? first);
  const aliasName = alias ? unquote(alias) : null;
  return {
    namespace: second ? unquote(first) : null,
    table,
    alias: aliasName && !NOT_ALIAS.has(aliasName.toLowerCase()) ? aliasName : null,
  };
}

/** 语句里 FROM / JOIN / UPDATE / INTO 后面的表,连同别名。FROM 后面逗号分开的多张表都算。 */
export function referencedTables(sql: string): TableReference[] {
  const text = stripLiterals(sql);
  const item = String.raw`${QUALIFIED}(?:\s+(?:as\s+)?(${NAME}))?`;
  const head = new RegExp(String.raw`\b(from|join|update|into)\s+${item}((?:\s*,\s*${item})*)`, "gi");
  const tail = new RegExp(String.raw`,\s*${item}`, "gi");
  const found: TableReference[] = [];
  for (const match of text.matchAll(head)) {
    found.push(reference(match[2]!, match[3], match[4]));
    if (match[1]!.toLowerCase() === "from" && match[5]) {
      for (const more of match[5].matchAll(tail)) found.push(reference(more[1]!, more[2], more[3]));
    }
  }
  const seen = new Set<string>();
  return found.filter((ref) => {
    const key = `${ref.namespace ?? ""}.${ref.table}.${ref.alias ?? ""}`.toLowerCase();
    if (seen.has(key) || /^\(/.test(ref.table)) return false;
    seen.add(key);
    return true;
  });
}

/** `x.` 里的 x 指的是哪张表:先认别名,再认表名(都不分大小写)。 */
export function resolveQualifier(qualifier: string, refs: readonly TableReference[]): TableReference | null {
  const lower = qualifier.toLowerCase();
  return (
    refs.find((ref) => ref.alias?.toLowerCase() === lower) ??
    refs.find((ref) => ref.table.toLowerCase() === lower) ??
    null
  );
}
