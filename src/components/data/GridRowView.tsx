import type { MouseEvent } from "react";
import type { ColumnInfo } from "../../lib/types";
import { CellEditor, type EditMove } from "./CellEditor";
import type { CellView, GridRow } from "./gridModel";

export interface EditingCell {
  row: number;
  col: number;
  seed?: string;
}

interface GridRowViewProps {
  gridId: string;
  row: GridRow;
  rowIndex: number;
  /** 行号槽里的文字:原有行是页内序号,新增行是 +。 */
  label: string;
  columns: readonly ColumnInfo[];
  views: readonly CellView[];
  top: number;
  selectedCol: number | null;
  editing: EditingCell | null;
  onCellMouseDown(row: number, col: number): void;
  onCellDoubleClick(row: number, col: number): void;
  onCellContextMenu(row: number, col: number, event: MouseEvent): void;
  onCommit(row: number, col: number, value: string, move: EditMove): void;
  onCancel(): void;
}

const STATUS_MARK: Record<GridRow["status"], string | null> = {
  clean: null,
  edited: "已修改",
  deleted: "将删除",
  new: "新增",
};

export const cellId = (gridId: string, row: number, col: number) => `${gridId}-r${row}-c${col}`;

export function GridRowView(props: GridRowViewProps) {
  const { gridId, row, rowIndex, label, columns, views, top, selectedCol, editing } = props;
  const status = STATUS_MARK[row.status];
  const classes = ["grid-row", `is-${row.status}`, selectedCol !== null && "has-selection", editing && "is-editing"];
  return (
    <div
      role="row"
      aria-rowindex={rowIndex + 2}
      className={classes.filter(Boolean).join(" ")}
      style={{ transform: `translateY(${top}px)` }}
    >
      <div role="rowheader" className="grid-gutter" title={status ?? undefined}>
        <span className="grid-gutter-mark" aria-hidden />
        <span className="grid-gutter-number">{label}</span>
        {status && <span className="visually-hidden">{status}</span>}
      </div>
      {views.map((view, col) => {
        const selected = selectedCol === col;
        const isEditing = editing?.col === col;
        const cellClasses = [
          "grid-cell",
          `is-${view.tone}`,
          view.numeric && "is-numeric",
          view.edited && "is-edited",
          selected && "is-selected",
          isEditing && "is-editing",
        ];
        return (
          <div
            key={col}
            id={cellId(gridId, rowIndex, col)}
            role="gridcell"
            aria-colindex={col + 2}
            aria-selected={selected}
            aria-readonly={!view.editable}
            className={cellClasses.filter(Boolean).join(" ")}
            onMouseDown={() => props.onCellMouseDown(rowIndex, col)}
            onDoubleClick={() => props.onCellDoubleClick(rowIndex, col)}
            onContextMenu={(event) => props.onCellContextMenu(rowIndex, col, event)}
          >
            <span className="grid-cell-text">{view.text}</span>
            {isEditing && (
              <CellEditor
                initial={view.editorText}
                seed={editing.seed}
                label={`编辑「${columns[col]!.name}」`}
                onCommit={(value, move) => props.onCommit(rowIndex, col, value, move)}
                onCancel={props.onCancel}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}
