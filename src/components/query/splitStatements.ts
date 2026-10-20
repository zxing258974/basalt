// 把编辑器里的 SQL 切成一条条语句。纯函数、按词法扫描:跳过字符串、带引号的标识符、注释、
// PostgreSQL 的 dollar 引号,认 MySQL 客户端的 DELIMITER;存储过程、触发器、BEGIN ATOMIC
// 这类带 BEGIN … END 的语句整体算一条。位置都是 UTF-16 下标,和 CodeMirror 一致。

export type SqlDialect = "mysql" | "postgres" | "sqlite";

export interface SqlStatement {
  /** 去掉首尾空白、不含分隔符的原文;语句前的注释算在里面。 */
  text: string;
  from: number;
  to: number;
  /** 分隔符之后(连同同一行跟着的行注释);最后一条没有分隔符时等于 to。 */
  end: number;
}

const isSpace = (ch: string | undefined) => ch !== undefined && /\s/.test(ch);
const isWordStart = (ch: string | undefined) => ch !== undefined && (/[A-Za-z_]/.test(ch) || ch > "\u007f");
const isWordPart = (ch: string | undefined) => ch !== undefined && (/[A-Za-z0-9_$]/.test(ch) || ch > "\u007f");

/** 从开引号起跳到闭引号之后。`backslash` 时反斜杠转义下一个字符;连写两个引号总是转义。 */
function skipQuoted(source: string, start: number, close: string, backslash: boolean): number {
  let i = start + 1;
  while (i < source.length) {
    const ch = source[i];
    if (backslash && ch === "\\") i += 2;
    else if (ch === close && source[i + 1] === close) i += 2;
    else if (ch === close) return i + 1;
    else i += 1;
  }
  return source.length;
}

/** 行注释到换行符为止(不含换行符)。 */
function skipLineComment(source: string, start: number): number {
  const newline = source.indexOf("\n", start);
  return newline === -1 ? source.length : newline;
}

function skipBlockComment(source: string, start: number, nested: boolean): number {
  let depth = 0;
  let i = start;
  while (i < source.length) {
    if (source.startsWith("/*", i)) {
      depth = nested || depth === 0 ? depth + 1 : depth;
      i += 2;
    } else if (source.startsWith("*/", i)) {
      depth -= 1;
      i += 2;
      if (depth === 0) return i;
    } else {
      i += 1;
    }
  }
  return source.length;
}

const DOLLAR_TAG = /^\$(?:[A-Za-z_\u0080-\uffff][A-Za-z0-9_\u0080-\uffff]*)?\$/;

/** PostgreSQL 的 $tag$ … $tag$;不是 dollar 引号(如 $1)时返回 null。 */
function skipDollarQuote(source: string, start: number): number | null {
  const tag = DOLLAR_TAG.exec(source.slice(start, start + 64))?.[0];
  if (!tag) return null;
  const close = source.indexOf(tag, start + tag.length);
  return close === -1 ? source.length : close + tag.length;
}

// 注释的范围;不是注释开头时返回 null。code 表示 MySQL 的「/*!」「/*+」注释,内容会被执行。
function commentAt(source: string, i: number, dialect: SqlDialect): { end: number; code: boolean } | null {
  const ch = source[i];
  const next = source[i + 1];
  if (ch === "-" && next === "-") {
    // MySQL 里 -- 后面必须跟空白或控制字符才是注释,否则是两个减号。
    const after = source[i + 2];
    if (dialect !== "mysql" || after === undefined || after <= " ")
      return { end: skipLineComment(source, i), code: false };
  }
  if (ch === "#" && dialect === "mysql") return { end: skipLineComment(source, i), code: false };
  if (ch === "/" && next === "*") {
    const marker = source[i + 2];
    const code = dialect === "mysql" && (marker === "!" || marker === "+");
    return { end: skipBlockComment(source, i, dialect === "postgres"), code };
  }
  return null;
}

/** 引号包起来的字符串或标识符的结尾;不是引号开头时返回 null。 */
function quotedAt(source: string, i: number, dialect: SqlDialect): number | null {
  const ch = source[i]!;
  if (ch === "'") return skipQuoted(source, i, "'", dialect === "mysql");
  if (ch === '"') return skipQuoted(source, i, '"', dialect === "mysql");
  if (ch === "`" && dialect !== "postgres") return skipQuoted(source, i, "`", false);
  if (ch === "[" && dialect === "sqlite") {
    const close = source.indexOf("]", i + 1);
    return close === -1 ? source.length : close + 1;
  }
  if (ch === "$" && dialect === "postgres") return skipDollarQuote(source, i);
  return null;
}

/** END 后面跟这些词时,关的是 IF / LOOP 之类,不是 BEGIN 块。 */
const NON_BLOCK_ENDS = new Set(["IF", "LOOP", "WHILE", "REPEAT"]);
const MYSQL_ROUTINES = new Set(["PROCEDURE", "FUNCTION", "TRIGGER", "EVENT"]);

/**
 * 跟踪 BEGIN … END:块里的分号不结束语句。只在会有语句体的语句里生效
 * (MySQL 的存储过程 / 函数 / 触发器 / 事件、SQLite 的触发器、PostgreSQL 的 BEGIN ATOMIC),
 * 单独一句 BEGIN 是开事务。
 */
class BlockTracker {
  private depth = 0;
  private caseDepth = 0;
  private pendingEnd = false;
  private words: string[] = [];
  private eligible = false;

  constructor(private readonly dialect: SqlDialect) {}

  onWord(word: string): void {
    if (this.pendingEnd) {
      this.pendingEnd = false;
      if (NON_BLOCK_ENDS.has(word)) return;
      if (word === "CASE") {
        this.caseDepth = Math.max(0, this.caseDepth - 1);
        return;
      }
      this.closeOne();
    }
    const previous = this.words.at(-1);
    this.words.push(word);
    this.eligible ||= this.startsBody(word);
    if (!this.eligible) return;
    if (this.opens(word, previous)) this.depth += 1;
    else if (word === "CASE") this.caseDepth += 1;
    else if (word === "END") this.pendingEnd = true;
  }

  /** 遇到分号:先把悬着的 END 算掉,再看是否在块外。 */
  canTerminate(): boolean {
    if (this.pendingEnd) {
      this.pendingEnd = false;
      this.closeOne();
    }
    return this.depth === 0;
  }

  private startsBody(word: string): boolean {
    const first = this.words[0];
    if (this.dialect === "mysql") {
      return (
        (first === "CREATE" && MYSQL_ROUTINES.has(word)) ||
        (first === "BEGIN" && this.words.length === 2 && word === "NOT")
      );
    }
    if (this.dialect === "sqlite") return first === "CREATE" && word === "TRIGGER";
    return first === "CREATE";
  }

  private opens(word: string, previous: string | undefined): boolean {
    if (this.dialect === "postgres") return word === "ATOMIC" && previous === "BEGIN";
    if (this.words[0] === "BEGIN" && this.depth === 0) return word === "ATOMIC" && previous === "NOT";
    return word === "BEGIN";
  }

  private closeOne(): void {
    if (this.caseDepth > 0) this.caseDepth -= 1;
    else if (this.depth > 0) this.depth -= 1;
  }
}

/** 一段语句的扫描状态:内容(含注释)的范围,以及有没有真正要执行的代码。 */
interface Segment {
  from: number;
  to: number;
  hasCode: boolean;
}

const emptySegment = (): Segment => ({ from: -1, to: -1, hasCode: false });

function mark(segment: Segment, from: number, to: number, code: boolean): void {
  if (segment.from === -1) segment.from = from;
  segment.to = to;
  segment.hasCode ||= code;
}

/** MySQL 客户端命令 `DELIMITER xx`:返回新分隔符和这一行的结尾;不是这个命令时返回 null。 */
function delimiterCommand(source: string, i: number): { delimiter: string; end: number } | null {
  if (source.slice(i, i + 9).toUpperCase() !== "DELIMITER" || !/[ \t]/.test(source[i + 9] ?? "")) return null;
  const end = skipLineComment(source, i);
  const delimiter =
    source
      .slice(i + 10, end)
      .trim()
      .split(/\s+/)[0] ?? "";
  return delimiter ? { delimiter, end } : null;
}

/** 分隔符之后同一行的空白和行注释算进上一条的尾巴。 */
function trailingEnd(source: string, after: number, dialect: SqlDialect): number {
  let j = after;
  while (source[j] === " " || source[j] === "\t") j += 1;
  const comment = commentAt(source, j, dialect);
  return comment && !source.startsWith("/*", j) ? comment.end : after;
}

/**
 * 读一个词(标识符或关键字);PostgreSQL 的 E'…' 连同字符串一起读完。
 * 自定义分隔符可能紧贴在词后面(END$$),碰到就停。
 */
function readWord(source: string, i: number, dialect: SqlDialect, delimiter: string): { word: string; end: number } {
  let end = i + 1;
  while (isWordPart(source[end]) && !(delimiter !== ";" && source.startsWith(delimiter, end))) end += 1;
  const word = source.slice(i, end).toUpperCase();
  if (dialect === "postgres" && word === "E" && source[end] === "'") {
    return { word: "", end: skipQuoted(source, end, "'", true) };
  }
  return { word, end };
}

export function splitStatements(source: string, dialect: SqlDialect): SqlStatement[] {
  const statements: SqlStatement[] = [];
  let delimiter = ";";
  let segment = emptySegment();
  let block = new BlockTracker(dialect);
  const finish = (end: number) => {
    if (segment.hasCode) {
      statements.push({ text: source.slice(segment.from, segment.to), from: segment.from, to: segment.to, end });
    }
    segment = emptySegment();
    block = new BlockTracker(dialect);
  };

  let i = 0;
  while (i < source.length) {
    const ch = source[i];
    if (isSpace(ch)) {
      i += 1;
      continue;
    }
    const command = dialect === "mysql" && !segment.hasCode ? delimiterCommand(source, i) : null;
    if (command) {
      delimiter = command.delimiter;
      segment = emptySegment();
      i = command.end;
      continue;
    }
    if (source.startsWith(delimiter, i) && (delimiter !== ";" || block.canTerminate())) {
      const end = trailingEnd(source, i + delimiter.length, dialect);
      finish(end);
      i = end;
      continue;
    }
    const comment = commentAt(source, i, dialect);
    if (comment) {
      mark(segment, i, comment.end, comment.code);
      i = comment.end;
      continue;
    }
    const quoted = quotedAt(source, i, dialect);
    if (quoted !== null) {
      mark(segment, i, quoted, true);
      i = quoted;
      continue;
    }
    if (isWordStart(ch) && !isWordPart(source[i - 1])) {
      const { word, end } = readWord(source, i, dialect, delimiter);
      // 限定名里的 t.end、s.begin 是标识符,不是关键字。
      if (word && source[i - 1] !== ".") block.onWord(word);
      mark(segment, i, end, true);
      i = end;
      continue;
    }
    mark(segment, i, i + 1, true);
    i += 1;
  }
  finish(segment.to);
  return statements;
}

/**
 * 光标所在的语句:在语句里或紧跟在分隔符后面就是它;落在语句之间时,
 * 同一行上前面有语句取前一条,否则取下一条,文末取最后一条。
 */
export function statementAtCursor(
  statements: readonly SqlStatement[],
  source: string,
  cursor: number,
): SqlStatement | null {
  const inside = statements.find((s) => s.from <= cursor && cursor <= s.end);
  if (inside) return inside;
  const before = statements.findLast((s) => s.end <= cursor);
  if (before && !source.slice(before.end, cursor).includes("\n")) return before;
  return statements.find((s) => s.from >= cursor) ?? before ?? null;
}
