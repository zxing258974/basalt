// 导出、转储写完文件之后对话框里的内容:写了什么、文件名,能在访达(Windows 上是资源管理器)里找到它。
// 只给正文和底栏,<Dialog> 由各自的对话框渲染:和选项页是同一个 <dialog>,换内容时不会重新弹出、抢焦点。
import { CircleCheck, FolderOpen } from "lucide-react";
import type { ReactNode } from "react";
import { getBackend } from "../../lib/api";
import type { ExportOutcome } from "../../lib/transferTypes";
import { errorMessage } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { fileManagerName, fileName } from "../../lib/platform";

/** `summary` 是大字的那一行,如「已导出 1,234 行，文件 96 KB」。 */
export function ExportDoneBody({ outcome, summary }: { outcome: ExportOutcome; summary: ReactNode }) {
  return (
    <div className="transfer-done">
      <CircleCheck className="transfer-done-icon" aria-hidden />
      <div>
        <p className="transfer-done-title">{summary}</p>
        <p className="transfer-done-path selectable" title={outcome.path}>
          {fileName(outcome.path)}
        </p>
        {outcome.notice && <p className="transfer-note">{outcome.notice}</p>}
      </div>
    </div>
  );
}

export function ExportDoneFooter({ path, onClose }: { path: string; onClose(): void }) {
  const reveal = async () => {
    try {
      await (await getBackend()).revealInFinder(path);
    } catch (error) {
      useUi.getState().notify(`没能在${fileManagerName()}中显示：${errorMessage(error)}`);
    }
  };
  return (
    <>
      <Button variant="secondary" icon={<FolderOpen />} onClick={() => void reveal()}>
        在{fileManagerName()}中显示
      </Button>
      <span className="footer-spacer" />
      <Button variant="primary" autoFocus onClick={onClose}>
        完成
      </Button>
    </>
  );
}
