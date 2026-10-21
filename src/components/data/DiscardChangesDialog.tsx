import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

/**
 * 确认框的标题和正文:表格修改、没保存的查询、没保存的表结构,有哪几样就说哪几样;
 * 退出应用时说「退出」,清空、改名表这类操作说它自己(`action`)。
 */
export function discardText(
  count: number,
  queries: number,
  designs: number,
  quit = false,
  action?: string,
): { title: string; message: string } {
  const parts = [
    count > 0 ? `${count} 行修改还没有提交` : null,
    queries > 0 ? `${queries} 个查询还没有保存` : null,
    designs > 0 ? `${designs} 张表的结构修改还没有保存` : null,
  ].filter((part) => part !== null);
  const onlyRows = queries === 0 && designs === 0;
  const onlyQueries = count === 0 && designs === 0;
  const title = onlyRows ? "放弃未提交的修改？" : onlyQueries ? "放弃未保存的查询？" : "放弃未保存的修改？";
  return { title, message: `有 ${parts.join("、")}，${verbOf(quit, action)}后就没有了。` };
}

function verbOf(quit: boolean | undefined, action: string | undefined): string {
  return quit ? "退出" : (action ?? "关闭");
}

/** 关闭标签页、连接或退出应用会丢掉未提交的表格修改、没保存的查询或表结构时的确认框。 */
export function DiscardChangesDialog() {
  const prompt = useUi((s) => s.discardPrompt);
  const askDiscard = useUi((s) => s.askDiscard);
  if (!prompt) return null;

  const cancel = () => {
    askDiscard(null);
    prompt.onCancel?.();
  };
  const text = discardText(prompt.count, prompt.queries, prompt.designs, prompt.quit, prompt.action);
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
            放弃修改并{verbOf(prompt.quit, prompt.action)}
          </Button>
        </>
      }
    >
      <p className="discard-message">{text.message}</p>
    </Dialog>
  );
}
