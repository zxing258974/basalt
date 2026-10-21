import { useState } from "react";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { errorMessage } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { secretStoreName } from "../../lib/platform";

export function DeleteConnectionDialog() {
  const pending = useUi((s) => s.pendingDelete);
  const confirmDelete = useUi((s) => s.confirmDelete);
  const { remove } = useConnectionActions();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  if (!pending) return null;

  const dismiss = () => {
    setError(undefined);
    confirmDelete(null);
  };

  const submit = async () => {
    setBusy(true);
    try {
      await remove(pending.id);
      dismiss();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      title={`删除连接「${pending.name}」?`}
      width={420}
      onClose={dismiss}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={dismiss}>
            取消
          </Button>
          <Button variant="danger" busy={busy} onClick={() => void submit()} autoFocus>
            删除
          </Button>
        </>
      }
    >
      <p className="dialog-text">只删除本机保存的连接配置和{secretStoreName()}里的密码，数据库本身不受影响。</p>
      {error && <p className="field-error">{error}</p>}
    </Dialog>
  );
}
