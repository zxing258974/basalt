import { useVirtualizer } from "@tanstack/react-virtual";
import { Ban, Copy, Eraser, Pencil, RotateCcw, Trash2, Undo2 } from "lucide-react";
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import type { EditInput, SortSpec, TablePage } from "../../lib/types";
import { ContextMenu, type MenuEntry } from "../ui/ContextMenu";
import type { EditMove } from "./CellEditor";
import type { EditState, RowTarget } from "./editState";
import { GridHeader } from "./GridHeader";
import { GridInput } from "./GridInput";
import { cellId, GridRowView, type EditingCell } from "./GridRowView";
import { cellView, gridRows, initialWidth, type GridRow, type GridSelection } from "./gridModel";

/** 行高、表头高度、行号槽宽度:虚拟滚动按这些数字算位置,同时作为 CSS 变量下发。 */
const ROW_HEIGHT = 26;
const HEADER_HEIGHT = 40;
const GUTTER_WIDTH = 56;
/** 估初始列宽时看前多少行。 */
const WIDTH_SAMPLE_ROWS = 50;

export interface GridActions {
  setCell(target: RowTarget, column: string, value: EditInput): void;
  revertCell(target: RowTarget, column: string): void;
  deleteRow(row: GridRow): void;
  restoreRow(row: GridRow): void;
}

interface DataGridProps {
  label: string;
  page: TablePage;
  edits: EditState;
  /** 本页第一行在整张表里的序号(从 0 开始),行号槽据此显示。 */
  rowOffset: number;
  editable: boolean;
  /** 必须落在 gridRows(page, edits) 的范围里:行数变少时由持有选区的一方用 fitSelection 收紧。 */
  selection: GridSelection | null;
  onSelectionChange(selection: GridSelection): void;
  sort: SortSpec | null;
  sortDisabled: boolean;
  onSort(column: string): void;
  actions: GridActions;
  /** 每次加一就让表格获得焦点(如点了「新增行」之后可以直接打字)。 */
  focusRequest: number;
  /** 换页、筛选、排序时变化,表格回到顶部;提交后重读不变,停在原处。 */
  scrollKey: string;
}

function measureWidths(page: TablePage): number[] {
  const sample = page.rows.slice(0, WIDTH_SAMPLE_ROWS);
  return page.columns.map((column, index) =>
    initialWidth(
      column,
      sample.map((row) => row[index]),
    ),
  );
}

export function DataGrid(props: DataGridProps) {
  const { page, edits, editable, selection, onSelectionChange, actions } = props;
  const gridId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  /** 组字开始时选中的格子:组字中途点了别的格子,文字仍然进开始时那一格。 */
  const composingAt = useRef<GridSelection | null>(null);
  const rows = useMemo(() => gridRows(page, edits), [page, edits]);
  const lastCol = page.columns.length - 1;

  // 表结构(列名)变了才重新估列宽;翻页、提交后保留用户拖过的宽度。
  const signature = page.columns.map((c) => c.name).join("\u0000");
  const [sized, setSized] = useState(() => ({ signature, widths: measureWidths(page) }));
  if (sized.signature !== signature) setSized({ signature, widths: measureWidths(page) });
  const widths = sized.signature === signature ? sized.widths : measureWidths(page);

  const [editing, setEditing] = useState<EditingCell | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; row: number; col: number } | null>(null);

  // 项目没启用 React Compiler;TanStack Virtual 的返回值本来就不该被 memo,这条提示对我们无影响。
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
    scrollPaddingStart: HEADER_HEIGHT,
  });

  const selectedRow = selection?.row;
  const selectedCol = selection?.col;
  useEffect(() => {
    if (selectedRow === undefined || selectedCol === undefined) return;
    virtualizer.scrollToIndex(selectedRow, { align: "auto" });
    const element = scrollRef.current;
    if (!element) return;
    // 行号槽是 sticky 的,左边要让出它的宽度。
    const left = GUTTER_WIDTH + widths.slice(0, selectedCol).reduce((sum, w) => sum + w, 0);
    const right = left + (widths[selectedCol] ?? 0);
    if (left - GUTTER_WIDTH < element.scrollLeft) element.scrollLeft = left - GUTTER_WIDTH;
    else if (right > element.scrollLeft + element.clientWidth) element.scrollLeft = right - element.clientWidth;
  }, [selectedRow, selectedCol, virtualizer, widths]);

  useEffect(() => {
    if (props.focusRequest > 0) inputRef.current?.focus({ preventScroll: true });
  }, [props.focusRequest]);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [props.scrollKey]);

  const viewOf = (row: number, col: number) => cellView(page, edits, rows[row]!, col);
  const focusGrid = () => inputRef.current?.focus({ preventScroll: true });
  const select = (row: number, col: number) => {
    if (rows.length === 0 || lastCol < 0) return;
    onSelectionChange({
      row: Math.min(Math.max(row, 0), rows.length - 1),
      col: Math.min(Math.max(col, 0), lastCol),
    });
  };

  const startEditing = (row: number, col: number, seed?: string) => {
    if (!editable || !viewOf(row, col).editable) return;
    select(row, col);
    setEditing({ row, col, seed });
  };

  const commit = (row: number, col: number, value: string, move: EditMove) => {
    actions.setCell(rows[row]!.target, page.columns[col]!.name, value);
    setEditing(null);
    select(row, move === "right" ? col + 1 : move === "left" ? col - 1 : col);
    focusGrid();
  };

  const copy = (row: number, col: number) => {
    const text = viewOf(row, col).editorText ?? "";
    navigator.clipboard?.writeText(text).catch((error: unknown) => console.warn("复制失败", error));
  };

  const removeRow = (row: number) => {
    const target = rows[row];
    if (!editable || !target) return;
    if (target.status === "deleted") actions.restoreRow(target);
    else actions.deleteRow(target);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (editing || rows.length === 0) return;
    // 输入法正在处理这个键(组字、挑候选、上屏):整个交给输入法,文字上屏后由 onText 接手。
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    const { row, col } = selection ?? { row: 0, col: 0 };
    const meta = event.metaKey || event.ctrlKey;
    const pageRows = Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? 0) / ROW_HEIGHT) - 2);
    const moves: Record<string, () => void> = {
      ArrowDown: () => select(meta ? rows.length - 1 : row + 1, col),
      ArrowUp: () => select(meta ? 0 : row - 1, col),
      ArrowRight: () => select(row, meta ? lastCol : col + 1),
      ArrowLeft: () => select(row, meta ? 0 : col - 1),
      Home: () => select(row, 0),
      End: () => select(row, lastCol),
      PageDown: () => select(row + pageRows, col),
      PageUp: () => select(row - pageRows, col),
      Enter: () => startEditing(row, col),
      F2: () => startEditing(row, col),
    };
    const action = moves[event.key];
    if (action && !(meta && (event.key === "Enter" || event.key === "F2"))) {
      event.preventDefault();
      if (!selection && event.key.startsWith("Arrow")) select(0, 0);
      else action();
    } else if (meta && (event.key === "Backspace" || event.key === "Delete")) {
      event.preventDefault();
      removeRow(row);
    } else if (meta && event.key.toLowerCase() === "c") {
      event.preventDefault();
      copy(row, col);
    }
    // 可打印的键不在这里拦:让字照常打进输入框,输入法才有机会组字。
  };

  const onText = (text: string, composed: boolean) => {
    const target = (composed ? composingAt.current : null) ?? selection ?? { row: 0, col: 0 };
    composingAt.current = null;
    // 输入框已经不是焦点(比如对话框打开后才上屏的字):打开的编辑框拿不到焦点,不如不开。
    if (editing || rows.length === 0 || document.activeElement !== inputRef.current) return;
    startEditing(target.row, target.col, text);
  };

  const menuItems = (row: number, col: number): MenuEntry[] => {
    const view = viewOf(row, col);
    const target = rows[row]!;
    const column = page.columns[col]!;
    const writable = editable && view.editable;
    return [
      { label: "编辑", icon: <Pencil />, shortcut: "↩", disabled: !writable, onSelect: () => startEditing(row, col) },
      {
        label: "设为 NULL",
        icon: <Ban />,
        disabled: !writable || !column.nullable,
        onSelect: () => actions.setCell(target.target, column.name, null),
      },
      {
        label: "设为空字符串",
        icon: <Eraser />,
        disabled: !writable,
        onSelect: () => actions.setCell(target.target, column.name, ""),
      },
      {
        label: "撤销这一格",
        icon: <Undo2 />,
        disabled: !view.edited,
        onSelect: () => actions.revertCell(target.target, column.name),
      },
      "separator",
      { label: "复制", icon: <Copy />, shortcut: "⌘C", onSelect: () => copy(row, col) },
      "separator",
      target.status === "deleted"
        ? { label: "恢复这一行", icon: <RotateCcw />, onSelect: () => actions.restoreRow(target) }
        : {
            label: target.status === "new" ? "移除新行" : "删除这一行",
            icon: <Trash2 />,
            shortcut: "⌘⌫",
            danger: true,
            disabled: !editable,
            onSelect: () => actions.deleteRow(target),
          },
    ];
  };

  const onCellContextMenu = (row: number, col: number, event: MouseEvent) => {
    event.preventDefault();
    select(row, col);
    setMenu({ x: event.clientX, y: event.clientY, row, col });
  };

  // 虚拟滚动会卸载看不见的行:选中行不在渲染范围里时不能指向一个不存在的 id。
  const virtualRows = virtualizer.getVirtualItems();
  const activeCell =
    selection && !editing && virtualRows.some((item) => item.index === selection.row)
      ? cellId(gridId, selection.row, selection.col)
      : undefined;

  // 输入框盖在选中格上(没有选区时在第一格),位置和编辑框打开时一致。
  const at = selection ?? { row: 0, col: 0 };
  const inputStyle: CSSProperties = {
    top: HEADER_HEIGHT + at.row * ROW_HEIGHT - 1,
    left: GUTTER_WIDTH + widths.slice(0, at.col).reduce((sum, w) => sum + w, 0) - 1,
    width: (widths[at.col] ?? 0) + 2,
    height: ROW_HEIGHT + 2,
  };
  const canType = editable && rows.length > 0 && at.row < rows.length && viewOf(at.row, at.col).editable;

  const style = {
    "--grid-columns": `${GUTTER_WIDTH}px ${widths.map((w) => `${w}px`).join(" ")}`,
    "--grid-width": `${GUTTER_WIDTH + widths.reduce((sum, w) => sum + w, 0)}px`,
    "--grid-row-height": `${ROW_HEIGHT}px`,
    "--grid-header-height": `${HEADER_HEIGHT}px`,
  } as CSSProperties;

  return (
    <>
      {/* 滚动容器可以被点中(tabIndex -1),但不占 Tab 位:焦点一到就转给输入框。 */}
      <div
        ref={scrollRef}
        className="data-grid"
        tabIndex={-1}
        style={style}
        onFocus={(event) => {
          if (event.target === event.currentTarget) focusGrid();
        }}
      >
        <GridInput
          ref={inputRef}
          label={props.label}
          controls={`${gridId}-grid`}
          activeCell={activeCell}
          editable={canType}
          style={inputStyle}
          onKeyDown={onKeyDown}
          onCompositionStart={() => {
            composingAt.current = selection;
          }}
          onText={onText}
        />
        <div
          id={`${gridId}-grid`}
          role="grid"
          className="grid-table"
          aria-label={props.label}
          aria-rowcount={rows.length + 1}
          aria-colcount={page.columns.length + 1}
        >
          <GridHeader
            columns={page.columns}
            widths={widths}
            sort={props.sort}
            sortDisabled={props.sortDisabled}
            onSort={props.onSort}
            onResize={(index, width) =>
              setSized((current) => ({ ...current, widths: current.widths.map((w, i) => (i === index ? width : w)) }))
            }
          />
          <div className="grid-body" role="rowgroup" style={{ height: virtualizer.getTotalSize() }}>
            {rows.length === 0 && <p className="grid-empty">没有符合条件的行</p>}
            {virtualRows.map((item) => {
              const row = rows[item.index]!;
              const number = row.target.kind === "existing" ? String(props.rowOffset + row.target.index + 1) : "+";
              return (
                <GridRowView
                  key={row.key}
                  gridId={gridId}
                  row={row}
                  rowIndex={item.index}
                  label={number}
                  columns={page.columns}
                  views={page.columns.map((_, col) => viewOf(item.index, col))}
                  top={item.start}
                  selectedCol={selection?.row === item.index ? selection.col : null}
                  editing={editing?.row === item.index ? editing : null}
                  onCellMouseDown={(r, c) => {
                    if (!editing) select(r, c);
                  }}
                  onCellDoubleClick={(r, c) => startEditing(r, c)}
                  onCellContextMenu={onCellContextMenu}
                  onCommit={commit}
                  onCancel={() => {
                    setEditing(null);
                    focusGrid();
                  }}
                />
              );
            })}
          </div>
        </div>
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems(menu.row, menu.col)}
          onClose={() => {
            setMenu(null);
            focusGrid();
          }}
        />
      )}
    </>
  );
}
