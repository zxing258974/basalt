// 单元格值在表格里怎么显示、在编辑框里是什么文本。
import type { CellValue, EditInput } from "../../lib/types";

/** 表格里一格最多显示的字符数,完整内容在编辑框里。 */
export const MAX_DISPLAY_CHARS = 300;
/** 字节在表格里最多显示的十六进制位数。 */
const MAX_DISPLAY_HEX = 64;

const NUMERIC_KINDS = new Set<CellValue["kind"]>(["int", "uint", "float", "decimal"]);

function shorten(text: string, limit: number): string {
  return text.length > limit ? `${text.slice(0, limit)}…` : text;
}

/** 单行显示:换行显示成 ↵,过长截断。 */
export function displayText(cell: CellValue): string {
  switch (cell.kind) {
    case "null":
      return "NULL";
    case "bool":
      return String(cell.value);
    case "unsupported":
      return `‹${cell.value.typeName}›`;
    case "bytes":
      return `0x${shorten(cell.value, MAX_DISPLAY_HEX)}`;
    default:
      return shorten(cell.value.replace(/\r?\n/g, "↵"), MAX_DISPLAY_CHARS);
  }
}

/** 编辑框里的原文:NULL 为 null,字节写成 0x 十六进制;解不了的类型没法编辑,为 undefined。 */
export function editorText(cell: CellValue): EditInput | undefined {
  switch (cell.kind) {
    case "null":
      return null;
    case "bool":
      return String(cell.value);
    case "unsupported":
      return undefined;
    case "bytes":
      return `0x${cell.value}`;
    default:
      return cell.value;
  }
}

/** 编辑后的值是否与原值一样;一样就不算修改。 */
export function sameAsOriginal(cell: CellValue, input: EditInput): boolean {
  return editorText(cell) === input;
}

export function isNumericCell(cell: CellValue): boolean {
  return NUMERIC_KINDS.has(cell.kind);
}
