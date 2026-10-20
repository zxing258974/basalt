import { ArrowDown, ArrowUp, KeyRound } from "lucide-react";
import { useRef, type PointerEvent } from "react";
import type { ColumnInfo, SortSpec } from "../../lib/types";
import { MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH } from "./gridModel";

interface GridHeaderProps {
  columns: readonly ColumnInfo[];
  widths: readonly number[];
  sort: SortSpec | null;
  /** 有未提交的修改时不能重新排序(会重读数据)。 */
  sortDisabled: boolean;
  onSort(column: string): void;
  onResize(index: number, width: number): void;
}

const clamp = (width: number) => Math.min(MAX_COLUMN_WIDTH * 2, Math.max(MIN_COLUMN_WIDTH, width));

export function GridHeader({ columns, widths, sort, sortDisabled, onSort, onResize }: GridHeaderProps) {
  const drag = useRef<{ index: number; startX: number; startWidth: number } | null>(null);

  const onPointerDown = (event: PointerEvent<HTMLSpanElement>, index: number) => {
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { index, startX: event.clientX, startWidth: widths[index] ?? MIN_COLUMN_WIDTH };
  };
  const onPointerMove = (event: PointerEvent<HTMLSpanElement>) => {
    const current = drag.current;
    if (current) onResize(current.index, clamp(current.startWidth + event.clientX - current.startX));
  };

  return (
    <div className="grid-header" role="row" aria-rowindex={1}>
      <div className="grid-gutter grid-gutter-head" role="columnheader" aria-label="行号" />
      {columns.map((column, index) => {
        const active = sort?.column === column.name;
        const ariaSort = active ? (sort.descending ? "descending" : "ascending") : "none";
        return (
          <div
            key={column.name}
            role="columnheader"
            aria-colindex={index + 2}
            aria-sort={ariaSort}
            className={active ? "grid-header-cell is-sorted" : "grid-header-cell"}
          >
            <button
              className="grid-sort-button"
              disabled={sortDisabled}
              title={sortDisabled ? "先提交或撤销修改再排序" : `按「${column.name}」排序`}
              // 鼠标点击不抢焦点:按钮获得焦点时浏览器会把它滚进视野,表格就横向跳走了。
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => onSort(column.name)}
            >
              <span className="grid-header-name">
                {column.isPrimaryKey && <KeyRound className="grid-key-icon" aria-label="主键" />}
                <span className="grid-header-label">{column.name}</span>
                {active && (sort.descending ? <ArrowDown aria-hidden /> : <ArrowUp aria-hidden />)}
              </span>
              <span className="grid-header-type">{column.dataType || "—"}</span>
            </button>
            <span
              className="grid-resizer"
              role="separator"
              aria-orientation="vertical"
              aria-label={`调整「${column.name}」列宽`}
              onPointerDown={(event) => onPointerDown(event, index)}
              onPointerMove={onPointerMove}
              onPointerUp={() => (drag.current = null)}
              onDoubleClick={(event) => event.stopPropagation()}
            />
          </div>
        );
      })}
    </div>
  );
}

/** 点列头:升序 → 降序 → 不排序(回到按主键)。 */
export function nextSort(current: SortSpec | null, column: string): SortSpec | null {
  if (current?.column !== column) return { column, descending: false };
  return current.descending ? null : { column, descending: true };
}
