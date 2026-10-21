import { useTransfer } from "../../stores/transfer";
import { ExportDialog } from "./ExportDialog";
import { ImportWizard } from "./ImportWizard";

/** 挂在应用最外层:同一时间最多开着一个导入或导出对话框。 */
export function TransferDialogs() {
  const dialog = useTransfer((s) => s.dialog);
  const close = useTransfer((s) => s.close);
  if (!dialog) return null;
  return dialog.kind === "export" ? (
    <ExportDialog target={dialog.target} onClose={close} />
  ) : (
    <ImportWizard target={dialog.target} onClose={close} />
  );
}
