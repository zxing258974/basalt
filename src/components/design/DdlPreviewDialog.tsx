import { CircleAlert } from "lucide-react";
import { useEffect } from "react";
import type { DdlPreview, TableDesign } from "../../lib/designTypes";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { DdlStatements, modeDescription } from "./DdlStatements";
import { hasPrimaryModifier, shortcut } from "../../lib/platform";

export type DdlPreviewState =
  | { phase: "closed" }
  | { phase: "loading" }
  /** 生成 DDL 就失败了(没填表名、字段重名……),改完再来。 */
  | { phase: "invalid"; message: string }
  /** `design` 是预览时用的那份草稿,执行的就是它,不会混进打开预览之后的改动。 */
  | (DdlPreview & {
      phase: "ready";
      design: TableDesign;
      applying: boolean;
      error: string | null;
      /** MySQL 中途失败:前面的语句已经生效。 */
      partial: boolean;
    });

interface DdlPreviewDialogProps {
  state: DdlPreviewState;
  onCancel(): void;
  onSubmit(): void;
}

export function DdlPreviewDialog({ state, onCancel, onSubmit }: DdlPreviewDialogProps) {
  const ready = state.phase === "ready" ? state : null;
  const applying = ready?.applying ?? false;
  const count = ready?.statements.length ?? 0;
  const canSubmit = ready !== null && !applying && count > 0 && !ready.partial;

  useEffect(() => {
    if (!canSubmit) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Enter" && hasPrimaryModifier(event)) {
        event.preventDefault();
        onSubmit();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [canSubmit, onSubmit]);

  if (state.phase === "closed") return null;

  const subtitle = ready
    ? count === 0
      ? "和库里的结构比，没有要执行的修改。"
      : modeDescription(ready.mode, count)
    : state.phase === "invalid"
      ? "表结构里有问题，什么都没有执行。"
      : "正在生成将要执行的 SQL…";

  return (
    <Dialog
      open
      title="保存表结构"
      subtitle={subtitle}
      width={760}
      onClose={() => {
        if (!applying) onCancel();
      }}
      footer={
        <>
          <span className="dialog-footer-hint">{canSubmit && `${shortcut("Enter")} 执行`}</span>
          <Button variant="secondary" disabled={applying} onClick={onCancel}>
            {state.phase === "invalid" ? "返回修改" : ready?.partial ? "关闭" : "取消"}
          </Button>
          {state.phase !== "invalid" && !ready?.partial && (
            <Button variant="danger" busy={applying} disabled={!canSubmit && !applying} onClick={onSubmit}>
              执行
            </Button>
          )}
        </>
      }
    >
      <div className="preview-body">
        {state.phase === "invalid" && <DdlError title="无法生成 SQL" message={state.message} />}
        {ready?.error && <DdlError title={ready.partial ? "部分修改已经生效" : "执行失败"} message={ready.error} />}
        {state.phase === "loading" && (
          <p className="preview-loading">
            <span className="spinner" aria-hidden /> 正在生成…
          </p>
        )}
        {ready && count > 0 && <DdlStatements statements={ready.statements} />}
      </div>
    </Dialog>
  );
}

function DdlError({ title, message }: { title: string; message: string }) {
  return (
    <div className="preview-error" role="alert">
      <CircleAlert aria-hidden />
      <div>
        <p className="preview-error-title">{title}</p>
        <p className="preview-error-message selectable">{message}</p>
      </div>
    </div>
  );
}
