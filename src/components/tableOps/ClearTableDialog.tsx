import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { tableDataKeys } from "../../hooks/useTableData";
import { getBackend } from "../../lib/api";
import { errorMessage, type ClearMode, type ConnectionView, type TableRef } from "../../lib/types";
import { useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ChoiceList } from "./ChoiceList";
import { useDetachable } from "./useDetachable";
import {
  clearChoices,
  clearedNotice,
  pendingWarning,
  pendingWork,
  qualifiedName,
  referenceNotes,
  tabsOfTable,
  truncateRefusal,
} from "./tableOpsModel";

interface ClearTableDialogProps {
  connection: ConnectionView;
  table: TableRef;
  /** 打开时选中的做法:菜单里点的是「清空表」还是「截断表」。 */
  mode: ClearMode;
  onClose(): void;
}

/**
 * 清空或截断表:两种做法放在一起,讲清楚区别。确认按钮不默认聚焦:回车不该清掉一张表。
 * 大表的 DELETE 要很久:执行中可以「在后台继续」,做完在状态栏提示。
 */
export function ClearTableDialog({ connection, table, mode: initialMode, onClose }: ClearTableDialogProps) {
  const queryClient = useQueryClient();
  const [mode, setMode] = useState(initialMode);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const { detach, isDetached } = useDetachable(onClose);
  const choices = clearChoices(connection.driver);
  const choice = choices.find((c) => c.mode === mode) ?? choices[0]!;
  const hasData = useTabs((s) => s.tabs.some((t) => t.kind === "tableData" && tabsOfTable(t, connection.id, table)));
  // 表结构不变,设计器里没保存的修改不受影响;数据页重读后未提交的修改就没了。
  const warning = useTabs((s) => {
    const work = pendingWork(s, connection.id, table);
    return pendingWarning({ rows: work.rows, designs: 0 }, choice.label);
  });
  // 别的表上引用它的外键:清空会按规则连带改动那些表,截断会被拒绝。
  const references = useQuery({
    queryKey: catalogKeys.references(connection.id, table.database, table.schema, table.table),
    queryFn: async () => (await getBackend()).tableReferences(connection.id, table),
    staleTime: 0,
  });
  const keys = references.data ?? [];
  const notes = referenceNotes(table, keys, mode, connection.driver);

  const run = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const outcome = await (await getBackend()).clearTable(connection.id, table, mode);
      // 按外键连带改动的表,开着的数据页也要重读。
      const touched = [
        table,
        ...keys.filter((k) => k.onDelete !== "restrict" && k.onDelete !== "noAction").map((k) => k.table),
      ];
      await Promise.all([
        ...touched.map((t) => queryClient.invalidateQueries({ queryKey: tableDataKeys.table(connection.id, t) })),
        queryClient.invalidateQueries({
          queryKey: catalogKeys.tables(connection.id, table.database, table.schema),
        }),
        queryClient.invalidateQueries({
          queryKey: catalogKeys.details(connection.id, table.database, table.schema, table.table),
        }),
      ]);
      useUi.getState().notify(clearedNotice(table, mode, outcome));
      if (!isDetached()) onClose();
    } catch (e) {
      const message = errorMessage(e);
      if (isDetached()) useUi.getState().notify(`${choice.label}表「${table.table}」失败：${message}`);
      else setError(mode === "truncate" ? truncateRefusal(message, table, keys) : message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      title={`${choice.label}表「${table.table}」？`}
      subtitle={`${connection.name} · ${qualifiedName(table)}`}
      width={500}
      onClose={busy ? detach : onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" autoFocus onClick={busy ? detach : onClose}>
            {busy ? "在后台继续" : "取消"}
          </Button>
          <Button variant="danger" busy={busy} onClick={() => void run()}>
            {choice.label}
          </Button>
        </>
      }
    >
      <div className="table-ops-form">
        <ChoiceList
          label="做法"
          choices={choices.map((c) => ({ value: c.mode, label: c.label, code: c.sql, detail: c.detail }))}
          value={mode}
          disabled={busy}
          onChange={(next) => {
            setMode(next);
            setError(undefined);
          }}
        />
        <p className="dialog-text">
          表里的全部行都会删掉，不能撤销；表结构、索引都留着。
          {hasData ? "开着的数据页会重新读取。" : ""}
        </p>
        {notes.warning && (
          <p className="transfer-warning" aria-label="外键">
            {notes.warning}
          </p>
        )}
        {notes.note && <p className="dialog-text is-muted">{notes.note}</p>}
        {references.error && (
          <p className="dialog-text is-muted">没能查到别的表上引用它的外键：{errorMessage(references.error)}</p>
        )}
        {warning && <p className="transfer-warning">{warning}</p>}
      </div>
      {error && (
        <p className="field-error table-ops-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
