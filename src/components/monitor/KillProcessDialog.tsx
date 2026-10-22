import { useState } from "react";
import { getBackend } from "../../lib/api";
import type { KillMode, ServerProcess } from "../../lib/monitorTypes";
import { errorMessage } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

interface KillProcessDialogProps {
  connectionId: string;
  process: ServerProcess;
  mode: KillMode;
  onClose(): void;
  /** 终止之后(成功或者会话已经变了):马上重取一份快照。 */
  onDone(): void;
}

const EFFECT: Record<KillMode, string> = {
  query: "只停掉正在执行的这条语句，连接还留着；在事务里的话，这条语句的改动会撤销。",
  connection: "连接会被断开，没提交的事务会回滚。用这条连接的程序得自己重连。",
};

/** 终止会话前确认:确认按钮不默认聚焦,回车不该断掉别人的连接。 */
export function KillProcessDialog({ connectionId, process, mode, onClose, onDone }: KillProcessDialogProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const who = [process.user, process.host].filter(Boolean).join(" @ ");
  const subtitle = [who, process.database].filter(Boolean).join(" · ");
  const title = mode === "query" ? `终止会话 ${process.id} 的语句？` : `断开会话 ${process.id}？`;

  const run = async () => {
    setBusy(true);
    setError(undefined);
    try {
      // 带上确认时看到的样子:后端发现会话已经换了语句、结束了就不动手。
      const target = { id: process.id, mode, sessionStart: process.sessionStart, query: process.query };
      await (await getBackend()).killProcess(connectionId, target);
      useUi.getState().notify(mode === "query" ? `已终止会话 ${process.id} 的语句。` : `已断开会话 ${process.id}。`);
      onDone();
      onClose();
    } catch (e) {
      setError(errorMessage(e));
      // 多半是会话已经变了:列表也跟着刷新。
      onDone();
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      title={title}
      subtitle={subtitle || undefined}
      width={480}
      onClose={onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" autoFocus onClick={onClose}>
            取消
          </Button>
          <Button variant="danger" busy={busy} onClick={() => void run()}>
            {mode === "query" ? "终止语句" : "断开连接"}
          </Button>
        </>
      }
    >
      <p className="dialog-text">{EFFECT[mode]}</p>
      {process.query && <pre className="kill-query mono selectable">{process.query}</pre>}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
