import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

/** 关闭标签页或连接会丢掉未提交的表格修改时的确认框。 */
export function DiscardChangesDialog() {
  const prompt = useUi((s) => s.discardPrompt);
  const askDiscard = useUi((s) => s.askDiscard);
  if (!prompt) return null;

  const cancel = () => {
    askDiscard(null);
    prompt.onCancel?.();
  };
  return (
    <Dialog
      open
      title="放弃未提交的修改？"
      width={420}
      onClose={cancel}
      footer={
        <>
          <span className="dialog-footer-hint" />
          <Button variant="secondary" onClick={cancel}>
            取消
          </Button>
          <Button
            variant="danger"
            onClick={() => {
              askDiscard(null);
              prompt.onConfirm();
            }}
          >
            放弃修改并关闭
          </Button>
        </>
      }
    >
      <p className="discard-message">有 {prompt.count} 行修改还没有提交，关闭后就没有了。</p>
    </Dialog>
  );
}
