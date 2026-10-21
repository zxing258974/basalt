import { useTableOps } from "../../stores/tableOps";
import { CopyTableDialog } from "./CopyTableDialog";
import { DropTableDialog } from "./DropTableDialog";

/** 挂在应用最外层:同一时间最多开着一个删除表或复制表的对话框。 */
export function TableOpsDialogs() {
  const dialog = useTableOps((s) => s.dialog);
  const close = useTableOps((s) => s.close);
  if (!dialog) return null;
  const Component = dialog.kind === "drop" ? DropTableDialog : CopyTableDialog;
  return <Component connection={dialog.connection} table={dialog.table} onClose={close} />;
}
