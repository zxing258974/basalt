import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

/** 确认框的标题和正文:只有表格修改、只有没保存的查询、两样都有,说法不同。 */
export function discardText(count: number, queries: number): { title: string; message: string } {
  const parts = [
    count > 0 ? `${count} 行修改还没有提交` : null,
    queries > 0 ? `${queries} 个查询还没有保存` : null,
  ].filter((part) => part !== null);
  const title = queries === 0 ? "放弃未提交的修改？" : count === 0 ? "放弃未保存的查询？" : "放弃未保存的修改？";
  return { title, message: `有 ${parts.join("、")}，关闭后就没有了。` };
}

/** 关闭标签页或连接会丢掉未提交的表格修改或没保存的查询时的确认框。 */
export function DiscardChangesDialog() {
  const prompt = useUi((s) => s.discardPrompt);
  const askDiscard = useUi((s) => s.askDiscard);
  if (!prompt) return null;

  const cancel = () => {
    askDiscard(null);
    prompt.onCancel?.();
  };
  const text = discardText(prompt.count, prompt.queries);
  return (
    <Dialog
      open
      title={text.title}
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
      <p className="discard-message">{text.message}</p>
    </Dialog>
  );
}
