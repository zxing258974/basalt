import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import { errorMessage, type ConnectionView, type TableInfo, type TableRef } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { tableKey } from "../sidebar/treeKeys";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";
import { COPY_LIMITS, copiedNotice, copyName, qualifiedName } from "./tableOpsModel";

interface CopyTableDialogProps {
  connection: ConnectionView;
  table: TableRef;
  onClose(): void;
}

const CHOICES = [
  { withData: true, label: "结构和数据", detail: "建一张结构相同的新表，再把全部行复制过去。" },
  { withData: false, label: "仅结构", detail: "只建一张结构相同的空表。" },
];

/** 复制表:新表和原表在同一个库(schema)里。 */
export function CopyTableDialog({ connection, table, onClose }: CopyTableDialogProps) {
  const queryClient = useQueryClient();
  const tablesKey = catalogKeys.tables(connection.id, table.database, table.schema);
  const [name, setName] = useState(() =>
    copyName(
      table.table,
      (queryClient.getQueryData<TableInfo[]>(tablesKey) ?? []).map((t) => t.name),
    ),
  );
  const [withData, setWithData] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const submit = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const outcome = await (await getBackend()).copyTable(connection.id, table, name.trim(), withData);
      await queryClient.invalidateQueries({ queryKey: tablesKey });
      const { select, notify } = useUi.getState();
      select(tableKey(connection.id, table.database, table.schema, outcome.table.table));
      notify(copiedNotice(outcome));
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
      title="复制表"
      subtitle={`${connection.name} · ${qualifiedName(table)}`}
      width={480}
      onClose={busy ? () => undefined : onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            取消
          </Button>
          <Button variant="primary" busy={busy} disabled={!name.trim()} onClick={() => void submit()}>
            复制
          </Button>
        </>
      }
    >
      <form
        className="table-ops-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (name.trim() && !busy) void submit();
        }}
      >
        <Field label="新表名" value={name} autoFocus spellCheck={false} onValueChange={setName} />
        <div className="transfer-range" role="radiogroup" aria-label="复制内容">
          {CHOICES.map((choice) => (
            <button
              key={choice.label}
              type="button"
              role="radio"
              aria-checked={withData === choice.withData}
              className={withData === choice.withData ? "transfer-range-option is-checked" : "transfer-range-option"}
              onClick={() => setWithData(choice.withData)}
            >
              <span className="transfer-radio" aria-hidden />
              <span className="transfer-range-label">{choice.label}</span>
              <span className="transfer-range-text">{choice.detail}</span>
            </button>
          ))}
        </div>
        <p className="dialog-text is-muted">{COPY_LIMITS[connection.driver]}</p>
      </form>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
