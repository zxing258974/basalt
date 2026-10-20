import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "../ui/Button";

export const PAGE_SIZES = [100, 500, 1000] as const;

interface DataFooterProps {
  offset: number;
  rowCount: number;
  limit: number;
  hasMore: boolean;
  /** 有未提交的修改时不能翻页、改每页行数。 */
  locked: boolean;
  onPrevious(): void;
  onNext(): void;
  onLimitChange(limit: number): void;
  pending: number;
  /** 最近一次提交的结果。 */
  message: string | null;
  sql: string | null;
}

export function DataFooter(props: DataFooterProps) {
  const { offset, rowCount, limit, hasMore, locked, pending } = props;
  const range = rowCount === 0 ? "没有行" : `第 ${offset + 1}–${offset + rowCount} 行`;
  return (
    <footer className="data-footer">
      <div className="pager">
        <Button
          variant="ghost"
          size="sm"
          icon={<ChevronLeft />}
          aria-label="上一页"
          disabled={locked || offset === 0}
          onClick={props.onPrevious}
        />
        <span className="pager-range mono">{range}</span>
        <Button
          variant="ghost"
          size="sm"
          icon={<ChevronRight />}
          aria-label="下一页"
          disabled={locked || !hasMore}
          onClick={props.onNext}
        />
        <label className="pager-size">
          每页
          <select
            value={limit}
            disabled={locked}
            aria-label="每页行数"
            onChange={(event) => props.onLimitChange(Number(event.target.value))}
          >
            {PAGE_SIZES.map((size) => (
              <option key={size} value={size}>
                {size}
              </option>
            ))}
          </select>
          行
        </label>
      </div>
      {pending > 0 ? (
        <span className="data-footer-pending">
          {pending} 行有未提交的修改 · <kbd>⌘S</kbd> 预览并提交
        </span>
      ) : props.message ? (
        <span className="data-footer-message" role="status">
          {props.message}
        </span>
      ) : null}
      {props.sql && (
        <code className="data-footer-sql selectable" title={props.sql}>
          {props.sql.replace(/\s+/g, " ")}
        </code>
      )}
    </footer>
  );
}
