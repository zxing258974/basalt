import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import {
  errorMessage,
  type ConnectionView,
  type Maintenance,
  type MaintenanceOutcome,
  type TableRef,
} from "../../lib/types";
import { useUi } from "../../stores/ui";
import { formatBytes } from "../transfer/exportModel";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ChoiceList } from "./ChoiceList";
import {
  MAINTENANCE,
  formatDuration,
  levelLabel,
  maintenanceNotice,
  maintenanceTitle,
  qualifiedName,
} from "./tableOpsModel";
import { useDetachable } from "./useDetachable";

interface MaintainTableDialogProps {
  connection: ConnectionView;
  table: TableRef;
  onClose(): void;
}

/**
 * 维护表:先选做哪一项,执行完在同一个对话框里看数据库返回的消息。
 * 大表的 OPTIMIZE、VACUUM 要很久:执行中可以「在后台继续」,做完在状态栏提示。
 */
export function MaintainTableDialog({ connection, table, onClose }: MaintainTableDialogProps) {
  const queryClient = useQueryClient();
  const { detach, isDetached } = useDetachable(onClose);
  const choices = MAINTENANCE[connection.driver];
  const [operation, setOperation] = useState<Maintenance>(choices[0]!.operation);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [outcome, setOutcome] = useState<MaintenanceOutcome | null>(null);
  const subtitle = `${connection.name} · ${qualifiedName(table)}`;
  const labelOf = (op: Maintenance) => choices.find((c) => c.operation === op)?.label ?? "维护";

  const run = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const result = await (await getBackend()).maintainTable(connection.id, table, operation);
      if (isDetached()) useUi.getState().notify(maintenanceNotice(table, labelOf(operation), result));
      else setOutcome(result);
      // 分析、清理之后行数估算、大小都可能变了。
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: catalogKeys.details(connection.id, table.database, table.schema, table.table),
        }),
        queryClient.invalidateQueries({
          queryKey: catalogKeys.tables(connection.id, table.database, table.schema),
        }),
      ]);
    } catch (e) {
      const message = errorMessage(e);
      if (isDetached()) useUi.getState().notify(`「${table.table}」${labelOf(operation)}失败：${message}`);
      else setError(message);
    } finally {
      setBusy(false);
    }
  };

  if (outcome) {
    return (
      <Dialog
        open
        title={maintenanceTitle(labelOf(operation), outcome)}
        subtitle={subtitle}
        width={520}
        onClose={onClose}
        footer={
          <>
            <Button variant="secondary" onClick={() => setOutcome(null)}>
              再做一项
            </Button>
            <span className="footer-spacer" />
            <Button variant="primary" autoFocus onClick={onClose}>
              完成
            </Button>
          </>
        }
      >
        <MaintenanceResult outcome={outcome} wholeFile={connection.driver === "sqlite"} />
      </Dialog>
    );
  }

  return (
    <Dialog
      open
      title="维护表"
      subtitle={subtitle}
      width={520}
      onClose={busy ? detach : onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={busy ? detach : onClose}>
            {busy ? "在后台继续" : "取消"}
          </Button>
          <Button variant="primary" autoFocus busy={busy} onClick={() => void run()}>
            执行
          </Button>
        </>
      }
    >
      <ChoiceList
        label="维护项"
        choices={choices.map((c) => ({ value: c.operation, label: c.label, code: c.sql, detail: c.detail }))}
        value={operation}
        disabled={busy}
        onChange={(next) => {
          setOperation(next);
          setError(undefined);
        }}
      />
      {error && (
        <p className="field-error table-ops-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}

/** `wholeFile`:SQLite 量的是整个库文件,PostgreSQL 量的是表连同索引。 */
function MaintenanceResult({ outcome, wholeFile }: { outcome: MaintenanceOutcome; wholeFile: boolean }) {
  const { size } = outcome;
  return (
    <div className="maintenance-result">
      <code className="maintenance-sql selectable">{outcome.sql}</code>
      {outcome.messages.length > 0 ? (
        <ul className="maintenance-messages" aria-label="数据库返回的消息">
          {outcome.messages.map((message, index) => (
            <li key={index} className="maintenance-message">
              <span className={`maintenance-level is-${message.kind}`}>{levelLabel(message.level)}</span>
              <span className="maintenance-text selectable">{message.text}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="dialog-text">执行完了，数据库没有返回消息。</p>
      )}
      <p className="maintenance-meta">
        {size && (
          <span>
            {wholeFile ? "库文件" : "表连同索引"} <span className="mono">{formatBytes(size.before)}</span> →{" "}
            <span className="mono">{formatBytes(size.after)}</span>
            {" · "}
          </span>
        )}
        用时 {formatDuration(outcome.elapsedMs)}
      </p>
    </div>
  );
}
