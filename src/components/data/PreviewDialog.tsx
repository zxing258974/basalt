import { CircleAlert } from "lucide-react";
import { useEffect } from "react";
import type { RowChange } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { hasPrimaryModifier, shortcut } from "../../lib/platform";

export type PreviewState =
  | { phase: "closed" }
  | { phase: "loading" }
  /** 生成语句就失败了(输入格式不对、没有主键……),改完再来。 */
  | { phase: "invalid"; message: string }
  /** `changes` 是预览时用的那批修改,提交的就是它,不会混进打开预览之后的改动。 */
  | { phase: "ready"; statements: string[]; changes: RowChange[]; applying: boolean; error: string | null };

interface PreviewDialogProps {
  state: PreviewState;
  onCancel(): void;
  onSubmit(): void;
}

const VERB_TONE: Record<string, string> = { UPDATE: "is-update", INSERT: "is-insert", DELETE: "is-delete" };

/** 语句开头的动词按语义上色:改=余烬、增=绿、删=红。 */
function Statement({ sql }: { sql: string }) {
  const [verb = "", ...rest] = sql.split(" ");
  return (
    <code className="sql-code selectable">
      <span className={`sql-verb ${VERB_TONE[verb] ?? ""}`}>{verb}</span> {rest.join(" ")}
    </code>
  );
}

export function PreviewDialog({ state, onCancel, onSubmit }: PreviewDialogProps) {
  const ready = state.phase === "ready" ? state : null;
  const applying = ready?.applying ?? false;
  const count = ready?.statements.length ?? 0;
  const canSubmit = ready !== null && !applying && count > 0;

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

  return (
    <Dialog
      open
      title="提交修改"
      subtitle={
        ready
          ? `将在一个事务里执行下面 ${count} 条语句；任何一条失败都会整体回滚。`
          : state.phase === "invalid"
            ? "修改里有数据库无法接受的内容，什么都没有执行。"
            : "正在生成将要执行的 SQL…"
      }
      width={720}
      onClose={() => {
        if (!applying) onCancel();
      }}
      footer={
        <>
          <span className="dialog-footer-hint">{ready && !applying && `${shortcut("Enter")} 提交`}</span>
          <Button variant="secondary" disabled={applying} onClick={onCancel}>
            {state.phase === "invalid" ? "返回修改" : "取消"}
          </Button>
          {state.phase !== "invalid" && (
            <Button variant="primary" busy={applying} disabled={!canSubmit && !applying} onClick={onSubmit}>
              提交
            </Button>
          )}
        </>
      }
    >
      <div className="preview-body">
        {state.phase === "invalid" && <PreviewError title="无法生成 SQL" message={state.message} />}
        {ready?.error && <PreviewError title="提交失败" message={ready.error} />}
        {state.phase === "loading" && (
          <p className="preview-loading">
            <span className="spinner" aria-hidden /> 正在生成…
          </p>
        )}
        {ready && (
          <ol className="sql-list" aria-label="将要执行的语句">
            {ready.statements.map((sql, index) => (
              <li key={index} className="sql-item">
                <span className="sql-number mono">{index + 1}</span>
                <Statement sql={sql} />
              </li>
            ))}
          </ol>
        )}
      </div>
    </Dialog>
  );
}

function PreviewError({ title, message }: { title: string; message: string }) {
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
