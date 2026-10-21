import { Check, FileDown, Minus, Plus, RefreshCw, RotateCcw, Undo2, X } from "lucide-react";
import { Button } from "../ui/Button";

interface DataToolbarProps {
  filterDraft: string;
  appliedFilter: string | null;
  onFilterDraftChange(value: string): void;
  onApplyFilter(): void;
  onClearFilter(): void;
  /** 有未提交的修改时不能换筛选、刷新:都会重读数据。 */
  locked: boolean;
  refreshing: boolean;
  onRefresh(): void;
  editable: boolean;
  /** 选中的行已标记删除时,删除按钮变成恢复。 */
  selectedDeleted: boolean;
  canDelete: boolean;
  onAddRow(): void;
  onToggleDelete(): void;
  pending: number;
  onRevert(): void;
  onCommit(): void;
  /** 导出这张表(带着当前的筛选条件)。 */
  onExport(): void;
}

const LOCKED_HINT = "先提交（⌘S）或撤销未提交的修改";

export function DataToolbar(props: DataToolbarProps) {
  const { filterDraft, appliedFilter, locked, editable, pending } = props;
  const filtering = Boolean(appliedFilter);
  const draftChanged = filterDraft.trim() !== (appliedFilter ?? "");

  return (
    <div className="data-toolbar">
      <label className={filtering ? "filter-box is-applied" : "filter-box"} title={locked ? LOCKED_HINT : undefined}>
        <span className="filter-keyword" aria-hidden>
          WHERE
        </span>
        <input
          value={filterDraft}
          placeholder="筛选条件，如 status = 'paid' AND amount > 100，回车执行"
          aria-label="筛选条件（WHERE 子句）"
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          disabled={locked}
          onChange={(event) => props.onFilterDraftChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === "Enter") props.onApplyFilter();
            if (event.key === "Escape") props.onFilterDraftChange(appliedFilter ?? "");
          }}
        />
        {draftChanged && !locked && <kbd className="filter-hint">↩</kbd>}
        {filtering && (
          <button className="filter-clear" aria-label="清除筛选" disabled={locked} onClick={props.onClearFilter}>
            <X />
          </button>
        )}
      </label>

      <div className="data-actions">
        <Button
          variant="ghost"
          size="sm"
          icon={<RefreshCw />}
          aria-label="刷新"
          title={locked ? LOCKED_HINT : "重新读取这一页"}
          busy={props.refreshing}
          disabled={locked}
          onClick={props.onRefresh}
        />
        <Button
          variant="ghost"
          size="sm"
          icon={<FileDown />}
          title={pending > 0 ? "导出的是数据库里的数据，不含还没提交的修改" : "导出这张表或筛选后的结果"}
          onClick={props.onExport}
        >
          导出
        </Button>
        <span className="data-actions-divider" aria-hidden />
        <Button variant="ghost" size="sm" icon={<Plus />} disabled={!editable} onClick={props.onAddRow}>
          新增行
        </Button>
        <Button
          variant="ghost"
          size="sm"
          icon={props.selectedDeleted ? <RotateCcw /> : <Minus />}
          disabled={!editable || !props.canDelete}
          title="⌘⌫"
          onClick={props.onToggleDelete}
        >
          {props.selectedDeleted ? "恢复行" : "删除行"}
        </Button>
        <span className="data-actions-divider" aria-hidden />
        <Button variant="ghost" size="sm" icon={<Undo2 />} disabled={pending === 0} onClick={props.onRevert}>
          撤销修改
        </Button>
        <Button
          variant={pending > 0 ? "primary" : "secondary"}
          size="sm"
          icon={<Check />}
          disabled={pending === 0}
          title="⌘S"
          onClick={props.onCommit}
        >
          提交{pending > 0 && <span className="commit-count">{pending}</span>}
        </Button>
      </div>
    </div>
  );
}
