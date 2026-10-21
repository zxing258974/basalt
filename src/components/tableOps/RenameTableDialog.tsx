import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import { errorMessage, type ConnectionView, type TableRef } from "../../lib/types";
import { useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { tableKey } from "../sidebar/treeKeys";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { RENAME_NOTES, pendingWarning, pendingWork, qualifiedName, renamedTab } from "./tableOpsModel";

interface RenameTableDialogProps {
  connection: ConnectionView;
  table: TableRef;
  onClose(): void;
}

/** 改名后,这张表开着的数据页和设计器换成新名字的标签页,对象树里选中新名字。 */
function followRename(connectionId: string, table: TableRef, name: string) {
  const { tabs, replace } = useTabs.getState();
  for (const tab of tabs) {
    const next = renamedTab(tab, connectionId, table, name);
    if (next) replace(tab.id, next);
  }
  const { selectedKey, select } = useUi.getState();
  if (selectedKey === tableKey(connectionId, table.database, table.schema, table.table)) {
    select(tableKey(connectionId, table.database, table.schema, name));
  }
}

/** 重命名表:留在同一个库(schema)里。 */
export function RenameTableDialog({ connection, table, onClose }: RenameTableDialogProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(table.table);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const warning = useTabs((s) => pendingWarning(pendingWork(s, connection.id, table), "改名"));
  const trimmed = name.trim();
  const ready = trimmed !== "" && trimmed !== table.table && !busy;

  const submit = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const renamed = await (await getBackend()).renameTable(connection.id, table, trimmed);
      followRename(connection.id, table, renamed.table);
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: catalogKeys.tables(connection.id, table.database, table.schema),
        }),
        queryClient.invalidateQueries({
          queryKey: catalogKeys.outline(connection.id, table.database, table.schema),
        }),
      ]);
      useUi.getState().notify(`已把「${table.table}」改名为「${renamed.table}」。`);
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      title="重命名表"
      subtitle={`${connection.name} · ${qualifiedName(table)}`}
      width={440}
      onClose={busy ? () => undefined : onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            取消
          </Button>
          <Button variant="primary" busy={busy} disabled={!ready} onClick={() => void submit()}>
            重命名
          </Button>
        </>
      }
    >
      <form
        className="table-ops-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) void submit();
        }}
      >
        <Field
          label="新表名"
          value={name}
          autoFocus
          spellCheck={false}
          onFocus={(event) => event.currentTarget.select()}
          onValueChange={setName}
        />
        <p className="dialog-text is-muted">
          {RENAME_NOTES[connection.driver]}程序里写死的表名也要自己改。开着的数据页和设计器会换成新名字。
        </p>
        {warning && <p className="transfer-warning">{warning}</p>}
      </form>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
