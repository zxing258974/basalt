import { useTableOps } from "../../stores/tableOps";
import { ClearTableDialog } from "./ClearTableDialog";
import { CopyTableDialog } from "./CopyTableDialog";
import { DropTableDialog } from "./DropTableDialog";
import { DumpTableDialog } from "./DumpTableDialog";
import { MaintainTableDialog } from "./MaintainTableDialog";
import { RenameTableDialog } from "./RenameTableDialog";

const DIALOGS = {
  drop: DropTableDialog,
  copy: CopyTableDialog,
  rename: RenameTableDialog,
  dump: DumpTableDialog,
  maintain: MaintainTableDialog,
};

/** 挂在应用最外层:同一时间最多开着一个表操作的对话框。 */
export function TableOpsDialogs() {
  const dialog = useTableOps((s) => s.dialog);
  const close = useTableOps((s) => s.close);
  if (!dialog) return null;
  if (dialog.kind === "clear") {
    return <ClearTableDialog connection={dialog.connection} table={dialog.table} mode={dialog.mode} onClose={close} />;
  }
  const Component = DIALOGS[dialog.kind];
  return <Component connection={dialog.connection} table={dialog.table} onClose={close} />;
}
