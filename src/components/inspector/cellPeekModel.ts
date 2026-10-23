// 信息面板「值」页的纯逻辑:选中的格子是什么、JSON 怎么排版、有多长。
import type { ColumnInfo, TablePage } from "../../lib/types";
import type { CellPeek, PeekValue } from "../../stores/cellPeek";
import { editedValue, type EditState } from "../data/editState";
import { cellView, type GridRow, type GridSelection } from "../data/gridModel";

/** 选中的那一格的完整内容;没选中为 null。`rowOffset` 同表格:本页第一行在整张表里的序号。 */
export function peekCell(
  page: TablePage,
  edits: EditState,
  rows: readonly GridRow[],
  selection: GridSelection | null,
  where: { source: string; rowOffset: number },
): CellPeek | null {
  const row = selection ? rows[selection.row] : undefined;
  const column = selection ? page.columns[selection.col] : undefined;
  if (!selection || !row || !column) return null;
  const pending = editedValue(edits, row.target, column.name);
  return {
    source: where.source,
    column: column.name,
    dataType: column.dataType,
    rowNumber: row.target.kind === "existing" ? where.rowOffset + row.target.index + 1 : null,
    value: pending === undefined ? storedValue(page, edits, row, selection.col) : editedPeek(pending, column),
    edited: pending !== undefined,
    deleted: row.status === "deleted",
  };
}

/** 二进制列里输入的 0x 十六进制按字节看,和写回时一样。 */
function editedPeek(value: string | null, column: ColumnInfo): PeekValue {
  if (value === null) return { kind: "null" };
  const hex = column.valueType.kind === "binary" ? /^0x([0-9a-f]*)$/i.exec(value) : null;
  return hex ? { kind: "bytes", hex: hex[1]! } : { kind: "text", text: value };
}

function storedValue(page: TablePage, edits: EditState, row: GridRow, col: number): PeekValue {
  const cell = row.original?.[col];
  if (!cell) return { kind: "unset", hint: cellView(page, edits, row, col).text };
  switch (cell.kind) {
    case "null":
      return { kind: "null" };
    case "bool":
      return { kind: "text", text: String(cell.value) };
    case "bytes":
      return { kind: "bytes", hex: cell.value };
    case "unsupported":
      return { kind: "unsupported", typeName: cell.value.typeName };
    default:
      return { kind: "text", text: cell.value };
  }
}

const JSON_SPACE = /[ \t\n\r]/;
const INDENT = "  ";
/** 超过这么长就不排版:校验要建出整棵树,排版再扫一遍,几 MB 的值每选一次都要卡一下。 */
export const MAX_FORMAT_CHARS = 1_000_000;
/** 嵌套超过这么深就不排版:缩进随深度增长,几千层能排出几千万字。MySQL 的 JSON 最多 100 层。 */
const MAX_JSON_DEPTH = 100;

/**
 * 是 JSON 对象或数组就排成两格缩进的多行,否则 null。只动字符串以外的空白:
 * 数字、字符串照原文抄,不经 JSON.parse 再写出来——超过 2^53 的整数、1.0、转义写法都不会被改掉。
 * 太长、嵌套太深的也是 null,照原文显示。
 */
export function prettyJson(text: string): string | null {
  if (text.length > MAX_FORMAT_CHARS) return null;
  // 只去掉 JSON 认的空白:BOM、不换行空格这些 JSON.parse 不认,留着就照原文显示,不被悄悄删掉。
  const source = text.replace(/^[ \t\n\r]+|[ \t\n\r]+$/g, "");
  if (!/^[[{]/.test(source)) return null;
  try {
    JSON.parse(source);
  } catch {
    return null;
  }
  let out = "";
  let depth = 0;
  const newline = () => `\n${INDENT.repeat(depth)}`;
  for (let i = 0; i < source.length; i += 1) {
    const ch = source[i]!;
    if (ch === '"') {
      let end = i + 1;
      while (source[end] !== '"') end += source[end] === "\\" ? 2 : 1;
      out += source.slice(i, end + 1);
      i = end;
    } else if (ch === "{" || ch === "[") {
      let next = i + 1;
      while (JSON_SPACE.test(source[next] ?? "")) next += 1;
      if (source[next] === (ch === "{" ? "}" : "]")) {
        out += ch + source[next]!;
        i = next;
      } else {
        depth += 1;
        if (depth > MAX_JSON_DEPTH) return null;
        out += ch + newline();
      }
    } else if (ch === "}" || ch === "]") {
      depth -= 1;
      out += newline() + ch;
    } else if (ch === ",") {
      out += `,${newline()}`;
    } else if (ch === ":") {
      out += ": ";
    } else if (!JSON_SPACE.test(ch)) {
      out += ch;
    }
  }
  return out;
}

/** 字数按字算(emoji 算一个),行数按换行算。 */
export function textStats(text: string): { chars: number; lines: number } {
  let chars = 0;
  let lines = 1;
  for (const ch of text) {
    chars += 1;
    if (ch === "\n") lines += 1;
  }
  return { chars, lines };
}

export type JsonTokenKind = "key" | "string" | "number" | "literal" | "punct" | "space";

export interface JsonToken {
  kind: JsonTokenKind;
  text: string;
}

/** 排好版的 JSON(prettyJson 的结果)切成上色用的几类:键、字符串、数字、true/false/null、标点、空白。 */
export function jsonTokens(pretty: string): JsonToken[] {
  const tokens: JsonToken[] = [];
  let i = 0;
  const take = (kind: JsonTokenKind, end: number) => {
    tokens.push({ kind, text: pretty.slice(i, end) });
    i = end;
  };
  const scan = (from: number, test: RegExp) => {
    let end = from;
    while (end < pretty.length && test.test(pretty[end]!)) end += 1;
    return end;
  };
  while (i < pretty.length) {
    const ch = pretty[i]!;
    if (JSON_SPACE.test(ch)) {
      take("space", scan(i, JSON_SPACE));
    } else if (ch === '"') {
      let end = i + 1;
      while (end < pretty.length && pretty[end] !== '"') end += pretty[end] === "\\" ? 2 : 1;
      end = Math.min(end + 1, pretty.length);
      take(pretty[scan(end, JSON_SPACE)] === ":" ? "key" : "string", end);
    } else if (/[-0-9]/.test(ch)) {
      take("number", scan(i, /[-+0-9.eE]/));
    } else if (/[a-z]/.test(ch)) {
      take("literal", scan(i, /[a-z]/));
    } else {
      take("punct", i + 1);
    }
  }
  return tokens;
}
