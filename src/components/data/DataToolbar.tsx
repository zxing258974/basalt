import { FileDown, RefreshCw, X } from "lucide-react";
import { Button } from "../ui/Button";
import { keyLabel, shortcut } from "../../lib/platform";
import { EditActions, type EditActionsProps } from "./EditActions";

interface DataToolbarProps extends EditActionsProps {
  filterDraft: string;
  appliedFilter: string | null;
  onFilterDraftChange(value: string): void;
  onApplyFilter(): void;
  onClearFilter(): void;
  /** 有未提交的修改时不能换筛选、刷新:都会重读数据。 */
  locked: boolean;
  refreshing: boolean;
  onRefresh(): void;
  /** 导出这张表(带着当前的筛选条件)。 */
  onExport(): void;
}

const lockedHint = () => `先提交（${shortcut("S")}）或撤销未提交的修改`;

export function DataToolbar(props: DataToolbarProps) {
  const { filterDraft, appliedFilter, locked, pending } = props;
  const filtering = Boolean(appliedFilter);
  const draftChanged = filterDraft.trim() !== (appliedFilter ?? "");

  return (
    <div className="data-toolbar">
      <label className={filtering ? "filter-box is-applied" : "filter-box"} title={locked ? lockedHint() : undefined}>
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
        {draftChanged && !locked && <kbd className="filter-hint">{keyLabel("Enter")}</kbd>}
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
          title={locked ? lockedHint() : "重新读取这一页"}
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
        <EditActions {...props} />
      </div>
    </div>
  );
}
