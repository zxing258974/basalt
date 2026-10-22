import { Check, Minus, Plus, RotateCcw, Undo2 } from "lucide-react";
import { Button } from "../ui/Button";
import { shortcut } from "../../lib/platform";

export interface EditActionsProps {
  editable: boolean;
  /** 能不能增删行;不填为能。连接了几张表的查询结果只能改格子,不放这两个按钮。 */
  rowEdits?: boolean;
  /** 选中的行已标记删除时,删除按钮变成恢复。 */
  selectedDeleted: boolean;
  canDelete: boolean;
  onAddRow(): void;
  onToggleDelete(): void;
  pending: number;
  onRevert(): void;
  onCommit(): void;
}

/** 新增、删除行,撤销、提交修改:表数据页的工具栏和可改的查询结果共用。 */
export function EditActions(props: EditActionsProps) {
  const { editable, pending } = props;
  return (
    <>
      {(props.rowEdits ?? true) && (
        <>
          <Button variant="ghost" size="sm" icon={<Plus />} disabled={!editable} onClick={props.onAddRow}>
            新增行
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={props.selectedDeleted ? <RotateCcw /> : <Minus />}
            disabled={!editable || !props.canDelete}
            title={shortcut("Backspace")}
            onClick={props.onToggleDelete}
          >
            {props.selectedDeleted ? "恢复行" : "删除行"}
          </Button>
          <span className="data-actions-divider" aria-hidden />
        </>
      )}
      <Button variant="ghost" size="sm" icon={<Undo2 />} disabled={pending === 0} onClick={props.onRevert}>
        撤销修改
      </Button>
      <Button
        variant={pending > 0 ? "primary" : "secondary"}
        size="sm"
        icon={<Check />}
        disabled={pending === 0}
        title={shortcut("S")}
        onClick={props.onCommit}
      >
        提交{pending > 0 && <span className="commit-count">{pending}</span>}
      </Button>
    </>
  );
}
