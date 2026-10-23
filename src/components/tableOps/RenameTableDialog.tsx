import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import { errorMessage, type ConnectionView, type TableRef } from "../../lib/types";
import { guardDiscard } from "../../stores/discard";
import { useTabs, type Tab } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { tableKey } from "../sidebar/treeKeys";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { forgetTable } from "./tableCaches";
import { RENAME_NOTES, driverNote, qualifiedName, renamedTab, tabsOfTable } from "./tableOpsModel";
import { useDetachable } from "./useDetachable";

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

/**
 * 重命名表:留在同一个库(schema)里。表被别的会话占着(比如查询页里没提交的事务)时要等锁:
 * 后端几秒就放弃,这期间也可以「在后台继续」,回去把那边的事务提交了。
 */
export function RenameTableDialog({ connection, table, onClose }: RenameTableDialogProps) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(table.table);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const { detach, isDetached } = useDetachable(onClose);
  const trimmed = name.trim();
  const ready = trimmed !== "" && trimmed !== table.table && !busy;
  const belongs = (tab: Tab) => tabsOfTable(tab, connection.id, table);

  const run = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const renamed = await (await getBackend()).renameTable(connection.id, table, trimmed);
      // 先清掉新名字可能留着的旧缓存(那个名字以前的表删了),标签页换过去才不会显示旧内容。
      forgetTable(queryClient, connection.id, renamed);
      followRename(connection.id, table, renamed.table);
      forgetTable(queryClient, connection.id, table);
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: catalogKeys.tables(connection.id, table.database, table.schema),
        }),
        queryClient.invalidateQueries({
          queryKey: catalogKeys.outline(connection.id, table.database, table.schema),
        }),
      ]);
      useUi.getState().notify(`已把「${table.table}」改名为「${renamed.table}」。`);
      if (!isDetached()) onClose();
    } catch (e) {
      const message = errorMessage(e);
      if (isDetached()) useUi.getState().notify(`把「${table.table}」改名失败：${message}`);
      else setError(message);
    } finally {
      setBusy(false);
    }
  };

  // 开着的数据页、设计器会换成新名字的标签页:里面没提交、没保存的修改先问一声。
  const submit = () => guardDiscard(belongs, () => void run(), undefined, "改名");

  return (
    <Dialog
      open
      title="重命名表"
      subtitle={`${connection.name} · ${qualifiedName(table)}`}
      width={440}
      onClose={busy ? detach : onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={busy ? detach : onClose}>
            {busy ? "在后台继续" : "取消"}
          </Button>
          <Button variant="primary" busy={busy} disabled={!ready} onClick={submit}>
            重命名
          </Button>
        </>
      }
    >
      <form
        className="table-ops-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) submit();
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
          {driverNote(RENAME_NOTES, connection.driver)}程序里写死的表名也要自己改。开着的数据页和设计器会换成新名字。
        </p>
      </form>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
