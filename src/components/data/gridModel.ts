// 表格的显示模型:哪些行、每格显示什么、能不能编辑。纯函数,不碰 DOM。
import type { CellValue, ColumnInfo, EditInput, TablePage } from "../../lib/types";
import { displayText, editorText, isNumericCell } from "./cellFormat";
import { editedValue, type EditState, type RowTarget } from "./editState";

export type RowStatus = "clean" | "edited" | "deleted" | "new";

export interface GridRow {
  key: string;
  target: RowTarget;
  /** 原有行读出来的值;新增行为 null。 */
  original: readonly CellValue[] | null;
  status: RowStatus;
}

export function gridRows(page: TablePage, edits: EditState): GridRow[] {
  const existing = page.rows.map((original, index): GridRow => ({
    key: `r${index}`,
    target: { kind: "existing", index },
    original,
    status: edits.deleted.includes(index) ? "deleted" : edits.edits[index] ? "edited" : "clean",
  }));
  const added = edits.inserted.map((row): GridRow => ({
    key: `n${row.id}`,
    target: { kind: "new", id: row.id },
    original: null,
    status: "new",
  }));
  return [...existing, ...added];
}

export type CellTone = "value" | "null" | "unsupported" | "placeholder";

export interface CellView {
  text: string;
  tone: CellTone;
  numeric: boolean;
  /** 有未提交的新值。 */
  edited: boolean;
  editable: boolean;
  /** 打开编辑框时的初始文本;NULL 为 null。 */
  editorText: EditInput;
}

function editedView(value: EditInput): Pick<CellView, "text" | "tone" | "editorText"> {
  return value === null
    ? { text: "NULL", tone: "null", editorText: null }
    : { text: displayText({ kind: "text", value }), tone: "value", editorText: value };
}

/** 新增行里没填的格子:提示提交后会是什么。 */
function placeholder(column: ColumnInfo): string {
  if (column.isGenerated) return "自动生成";
  if (column.hasDefault) return "默认";
  return column.nullable ? "NULL" : "必填";
}

export function cellView(page: TablePage, edits: EditState, row: GridRow, columnIndex: number): CellView {
  const column = page.columns[columnIndex]!;
  const pending = editedValue(edits, row.target, column.name);
  const writable = !column.isGenerated && row.status !== "deleted";
  if (pending !== undefined) {
    return { ...editedView(pending), numeric: false, edited: true, editable: writable };
  }
  const original = row.original?.[columnIndex];
  if (!original) {
    return {
      text: placeholder(column),
      tone: "placeholder",
      numeric: false,
      edited: false,
      editable: writable,
      editorText: "",
    };
  }
  // 文本列里出现的二进制值(无效 UTF-8、SQLite 动态类型)写回时 0x 不会被当十六进制,改了就变成字面文本。
  const bytesInTextColumn = original.kind === "bytes" && column.valueType.kind !== "binary";
  const text = bytesInTextColumn ? undefined : editorText(original);
  const tone: CellTone = original.kind === "null" ? "null" : original.kind === "unsupported" ? "unsupported" : "value";
  return {
    text: displayText(original),
    tone,
    numeric: isNumericCell(original),
    edited: false,
    editable: writable && text !== undefined,
    editorText: text ?? null,
  };
}

export const MIN_COLUMN_WIDTH = 72;
export const MAX_COLUMN_WIDTH = 360;
/** 等宽字体 12px 下每个字符的大致宽度;中文按两个字符算。 */
const CHAR_WIDTH = 7.2;
/** 左右内边距各 12px、右边框 1px,再留一点余量,免得刚好等宽时被省略号截掉。 */
const CELL_PADDING = 34;
/** 表头除了列名还有主键图标与排序箭头。 */
const HEADER_EXTRA = 36;

function visualLength(text: string): number {
  let length = 0;
  for (const char of text) length += char.charCodeAt(0) > 0x2e80 ? 2 : 1;
  return length;
}

/** 按列名和前几十行的值估一个初始宽度,之后用户可以拖动调整。 */
export function initialWidth(column: ColumnInfo, sample: readonly (CellValue | undefined)[]): number {
  const header = visualLength(column.name) * CHAR_WIDTH + HEADER_EXTRA;
  const longest = sample.reduce((max, cell) => (cell ? Math.max(max, visualLength(displayText(cell))) : max), 0);
  const content = longest * CHAR_WIDTH + CELL_PADDING;
  return Math.round(Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, header, content)));
}
