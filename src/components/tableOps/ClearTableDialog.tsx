import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { tableDataKeys } from "../../hooks/useTableData";
import { getBackend } from "../../lib/api";
import { errorMessage, type ClearMode, type ConnectionView, type TableRef } from "../../lib/types";
import { guardDiscard } from "../../stores/discard";
import { useTabs, type Tab } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ChoiceList } from "./ChoiceList";
import { useDetachable } from "./useDetachable";
import {
  clearChoices,
  clearRefusal,
  clearTouches,
  clearedNotice,
  qualifiedName,
  referenceNotes,
  rowsEstimate,
  tabsOfTable,
} from "./tableOpsModel";

interface ClearTableDialogProps {
  connection: ConnectionView;
  table: TableRef;
  /** 打开时选中的做法:菜单里点的是「清空表」还是「截断表」。 */
  mode: ClearMode;
  /** 对象列表里的行数(估算);数据库不给(SQLite)时为 null。 */
  rows: number | null;
  onClose(): void;
}

/**
 * 清空或截断表:两种做法放在一起,讲清楚区别。确认按钮不默认聚焦:回车不该清掉一张表。
 * 大表的 DELETE 要很久:执行中可以「在后台继续」,做完在状态栏提示。
 */
export function ClearTableDialog({ connection, table, mode: initialMode, rows, onClose }: ClearTableDialogProps) {
  const queryClient = useQueryClient();
  const [mode, setMode] = useState(initialMode);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const { detach, isDetached } = useDetachable(onClose);
  const choices = clearChoices(connection.driver);
  const choice = choices.find((c) => c.mode === mode) ?? choices[0]!;
  // 别的表上引用它的外键:清空会按规则连带改动那些表,截断会被拒绝。查完之前不让确认:
  // 看不到连带警告,也不知道哪些表的数据页要重读、里面没提交的修改要先问。
  const references = useQuery({
    queryKey: catalogKeys.references(connection.id, table.database, table.schema, table.table),
    queryFn: async () => (await getBackend()).tableReferences(connection.id, table),
    staleTime: 0,
  });
  const keys = references.data ?? [];
  const notes = referenceNotes(table, keys, mode, connection.driver);
  const touched = clearTouches(table, keys, mode, connection.driver);
  // 表结构不变,设计器里没保存的修改不受影响;这些表的数据页重读后,未提交的修改就没了。
  const isDataTab = (tab: Tab) => tab.kind === "tableData" && touched.some((t) => tabsOfTable(tab, connection.id, t));
  const hasData = useTabs((s) => s.tabs.some(isDataTab));

  const run = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const outcome = await (await getBackend()).clearTable(connection.id, table, mode);
      // 按外键连带改动的表,开着的数据页也要重读。
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
      const message = clearRefusal(errorMessage(e), mode, connection.driver, table, keys);
      if (isDetached())
        useUi.getState().notify(`${choice.label}表「${table.table}」失败：${message.replace("\n", " ")}`);
      else setError(message);
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
          <Button
            variant="danger"
            busy={busy}
            disabled={references.isPending}
            onClick={() => guardDiscard(isDataTab, () => void run(), undefined, choice.label)}
          >
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
          表里的全部行{rows === null ? "" : `（${rowsEstimate(rows)}）`}都会删掉，不能撤销。
        </p>
        <p className="dialog-text is-muted">表结构、索引都留着{hasData ? "；开着的数据页会重新读取" : ""}。</p>
        {notes.warning && (
          <p className="transfer-warning" aria-label="外键">
            {notes.warning}
          </p>
        )}
        {notes.note && <p className="dialog-text is-muted">{notes.note}</p>}
        {references.isPending && <p className="dialog-text is-muted">正在查别的表上引用它的外键…</p>}
        {references.error && (
          <p className="dialog-text is-muted">没能查到别的表上引用它的外键：{errorMessage(references.error)}</p>
        )}
      </div>
      {error && (
        <p className="field-error table-ops-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
