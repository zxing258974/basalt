// 表格里未提交的修改。纯函数、不可变更新:每次返回新对象,便于撤销和比较。
import type { CellValue, EditInput, RowChange, TablePage } from "../../lib/types";
import { sameAsOriginal } from "./cellFormat";

export interface NewRow {
  readonly id: number;
  /** 只有用户填过的列;没填的列提交时省略,由数据库取默认值。 */
  readonly values: Readonly<Record<string, EditInput>>;
}

export interface EditState {
  /** 原有行(按本页行号)→ 列名 → 新值。 */
  readonly edits: Readonly<Record<number, Readonly<Record<string, EditInput>>>>;
  /** 标记删除的原有行号。 */
  readonly deleted: readonly number[];
  readonly inserted: readonly NewRow[];
  readonly nextId: number;
}

export type RowTarget =
  | { readonly kind: "existing"; readonly index: number }
  | { readonly kind: "new"; readonly id: number };

export const EMPTY_EDITS: EditState = { edits: {}, deleted: [], inserted: [], nextId: 0 };

function withoutKey<T>(record: Readonly<Record<string, T>>, key: string): Record<string, T> {
  const { [key]: _removed, ...rest } = record;
  return rest;
}

function updateNewRow(state: EditState, id: number, update: (row: NewRow) => NewRow): EditState {
  return { ...state, inserted: state.inserted.map((row) => (row.id === id ? update(row) : row)) };
}

/** 原有行的某格改回原值时去掉这条修改,行里没有修改了就整行去掉。 */
function setExistingCell(state: EditState, index: number, column: string, value: EditInput | undefined): EditState {
  const current = state.edits[index] ?? {};
  const next = value === undefined ? withoutKey(current, column) : { ...current, [column]: value };
  const edits = { ...state.edits };
  if (Object.keys(next).length === 0) delete edits[index];
  else edits[index] = next;
  return { ...state, edits };
}

export function setCell(
  state: EditState,
  page: TablePage,
  target: RowTarget,
  column: string,
  value: EditInput,
): EditState {
  if (target.kind === "new") {
    return updateNewRow(state, target.id, (row) => ({ ...row, values: { ...row.values, [column]: value } }));
  }
  const columnIndex = page.columns.findIndex((c) => c.name === column);
  const original = page.rows[target.index]?.[columnIndex];
  const unchanged = original !== undefined && sameAsOriginal(original, value);
  return setExistingCell(state, target.index, column, unchanged ? undefined : value);
}

export function revertCell(state: EditState, target: RowTarget, column: string): EditState {
  if (target.kind === "new") {
    return updateNewRow(state, target.id, (row) => ({ ...row, values: withoutKey(row.values, column) }));
  }
  return setExistingCell(state, target.index, column, undefined);
}

/** 删除一行时丢掉它的修改:删的是数据库里的原行。 */
export function deleteRow(state: EditState, index: number): EditState {
  if (state.deleted.includes(index)) return state;
  const { [index]: _dropped, ...edits } = state.edits;
  return { ...state, edits, deleted: [...state.deleted, index] };
}

export function restoreRow(state: EditState, index: number): EditState {
  return { ...state, deleted: state.deleted.filter((i) => i !== index) };
}

export function addRow(state: EditState): EditState {
  return { ...state, inserted: [...state.inserted, { id: state.nextId, values: {} }], nextId: state.nextId + 1 };
}

export function removeNewRow(state: EditState, id: number): EditState {
  return { ...state, inserted: state.inserted.filter((row) => row.id !== id) };
}

/** 有修改的行数:改过的、删掉的、新增的各算一行。 */
export function pendingCount(state: EditState): number {
  return Object.keys(state.edits).length + state.deleted.length + state.inserted.length;
}

/** 这一格未提交的新值;没改过为 undefined。 */
export function editedValue(state: EditState, target: RowTarget, column: string): EditInput | undefined {
  const values = target.kind === "new" ? state.inserted.find((r) => r.id === target.id)?.values : state.edits[target.index];
  return values && column in values ? values[column] : undefined;
}

function keyOf(page: TablePage, index: number): Record<string, CellValue> {
  const row = page.rows[index] ?? [];
  return Object.fromEntries(
    page.columns.flatMap((column, i) => (column.isPrimaryKey && row[i] ? [[column.name, row[i]]] : [])),
  );
}

/** 按行号顺序给出修改与删除,新增排在最后。主键用读出来时的原值定位。 */
export function buildChanges(state: EditState, page: TablePage): RowChange[] {
  const touched = [...new Set([...Object.keys(state.edits).map(Number), ...state.deleted])].sort((a, b) => a - b);
  const existing = touched.map((index): RowChange =>
    state.deleted.includes(index)
      ? { kind: "delete", key: keyOf(page, index) }
      : { kind: "update", key: keyOf(page, index), values: { ...state.edits[index] } },
  );
  const inserts = state.inserted.map((row): RowChange => ({ kind: "insert", values: { ...row.values } }));
  return [...existing, ...inserts];
}
