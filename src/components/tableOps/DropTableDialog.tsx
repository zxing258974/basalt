import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import { errorMessage, type ConnectionView, type TableRef } from "../../lib/types";
import { guardDiscard } from "../../stores/discard";
import { useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { tableKey } from "../sidebar/treeKeys";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { forgetTable } from "./tableCaches";
import { qualifiedName, tabsOfTable } from "./tableOpsModel";
import { useDetachable } from "./useDetachable";

interface DropTableDialogProps {
  connection: ConnectionView;
  table: TableRef;
  onClose(): void;
}

/**
 * 删除表。确认按钮不默认聚焦:回车不该删掉一张表。表被别的会话占着时要等锁(后端几秒就放弃),
 * 这期间可以「在后台继续」。
 */
export function DropTableDialog({ connection, table, onClose }: DropTableDialogProps) {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const belongs = (tab: Parameters<typeof tabsOfTable>[0]) => tabsOfTable(tab, connection.id, table);
  const hasTabs = useTabs((s) => s.tabs.some(belongs));

  const { detach, isDetached } = useDetachable(onClose);

  const run = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await (await getBackend()).dropTable(connection.id, table);
      const { tabs, close } = useTabs.getState();
      for (const tab of tabs.filter(belongs)) close(tab.id);
      // 以后再有同名的表(复制、改名改成它),不能先显示这张被删的表的内容。
      forgetTable(queryClient, connection.id, table);
      const { selectedKey, select, notify } = useUi.getState();
      if (selectedKey === tableKey(connection.id, table.database, table.schema, table.table)) select(null);
      await queryClient.invalidateQueries({
        queryKey: catalogKeys.tables(connection.id, table.database, table.schema),
      });
      notify(`已删除表「${table.table}」。`);
      if (!isDetached()) onClose();
    } catch (e) {
      const message = errorMessage(e);
      if (isDetached()) useUi.getState().notify(`删除表「${table.table}」失败：${message}`);
      else setError(message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      title={`删除表「${table.table}」？`}
      subtitle={`${connection.name} · ${qualifiedName(table)}`}
      width={440}
      onClose={busy ? detach : onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" autoFocus onClick={busy ? detach : onClose}>
            {busy ? "在后台继续" : "取消"}
          </Button>
          <Button variant="danger" busy={busy} onClick={() => guardDiscard(belongs, () => void run())}>
            删除表
          </Button>
        </>
      }
    >
      <p className="dialog-text">
        表结构和里面的全部数据都会删掉，不能撤销。
        {hasTabs ? "开着的数据页和设计器会一起关掉。" : ""}
      </p>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
