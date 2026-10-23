import { useEffect, useMemo, useState } from "react";
import { usePublishCellPeek } from "../../hooks/usePublishCellPeek";
import { getBackend } from "../../lib/api";
import { hasPrimaryModifier } from "../../lib/platform";
import type { KeyDetail, RedisKind } from "../../lib/redisTypes";
import type { TablePage } from "../../lib/types";
import { DataGrid } from "../data/DataGrid";
import { EditActions } from "../data/EditActions";
import * as editing from "../data/editState";
import { fitSelection, gridRows, type GridSelection } from "../data/gridModel";
import { PreviewDialog } from "../data/PreviewDialog";
import { useGridEdits } from "../data/useGridEdits";
import { peekCell } from "../inspector/cellPeekModel";
import { formatLength } from "./redisKeyModel";

interface TableValueProps {
  connectionId: string;
  db: number;
  detail: KeyDetail;
  page: TablePage;
  truncated: boolean;
  active: boolean;
  onCommitted(): Promise<void>;
  onPendingChange(pending: number): void;
}

/** 新增一行时先填的那一格:list 是值,别的是第一列。 */
const FIRST_INPUT: Record<string, string> = { list: "value" };

/** hash、list、set、zset、stream 的值:表格里改、⌘S 预览命令、核对后一起提交。 */
export function TableValue(props: TableValueProps) {
  const { connectionId, db, detail, page } = props;
  const keyId = detail.key.id;
  const kind: RedisKind = detail.kind;
  const [selection, setSelection] = useState<GridSelection | null>(null);
  const { edits, setEdits, pending, actions, preview, openPreview, closePreview, submit, message, ...grid } =
    useGridEdits(page, page, {
      preview: async (changes) => (await getBackend()).redisPreviewKeyChanges(connectionId, keyId, kind, changes),
      apply: async (changes, statements) =>
        (await getBackend()).redisApplyKeyChanges(connectionId, db, keyId, kind, changes, statements),
      refresh: () => props.onCommitted(),
      wholeRowKeys: true,
      summary: (outcome) => `已执行 ${outcome.statements} 条命令。`,
    });
  const editable = page.readOnlyReason === null;
  const rows = useMemo(() => gridRows(page, edits), [page, edits]);
  const fitted = fitSelection(selection, rows.length, page.columns.length);
  if (fitted !== selection) setSelection(fitted);
  const selectedRow = fitted ? rows[fitted.row] : undefined;

  const peek = useMemo(
    () => peekCell(page, edits, rows, fitted, { source: detail.key.name, rowOffset: 0 }),
    [detail.key.name, edits, fitted, page, rows],
  );
  usePublishCellPeek(peek);

  const { onPendingChange, active } = props;
  useEffect(() => onPendingChange(pending), [onPendingChange, pending]);
  useEffect(() => () => onPendingChange(0), [onPendingChange]);

  useEffect(() => {
    if (!active) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (hasPrimaryModifier(event) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "s") {
        event.preventDefault();
        window.setTimeout(() => void openPreview(), 0);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, openPreview]);

  const addRow = () => {
    setEdits((current) => editing.addRow(current));
    const first = page.columns.findIndex((c) => c.name === (FIRST_INPUT[kind] ?? page.columns[0]?.name));
    setSelection({ row: rows.length, col: Math.max(first, 0) });
    grid.requestFocus();
  };

  return (
    <div className="key-value table-value">
      <div className="key-value-bar">
        <EditActions
          editable={editable}
          selectedDeleted={selectedRow?.status === "deleted"}
          canDelete={selectedRow !== undefined}
          onAddRow={addRow}
          onToggleDelete={() => {
            if (selectedRow?.status === "deleted") actions.restoreRow(selectedRow);
            else if (selectedRow) actions.deleteRow(selectedRow);
          }}
          pending={pending}
          onRevert={() => setEdits(editing.EMPTY_EDITS)}
          onCommit={() => void openPreview()}
        />
      </div>
      {page.readOnlyReason && <p className="key-value-note">{page.readOnlyReason}</p>}
      <DataGrid
        label={`${detail.key.name} 的值`}
        page={page}
        edits={edits}
        rowOffset={0}
        editable={editable}
        selection={fitted}
        onSelectionChange={setSelection}
        sort={null}
        sortDisabled
        onSort={() => {}}
        actions={actions}
        focusRequest={grid.focusRequest}
        scrollKey={keyId}
      />
      <footer className="key-value-footer">
        <span>
          {props.truncated
            ? `共 ${formatLength(kind, detail.length)}，只读出了前 ${page.rows.length.toLocaleString()} 个`
            : formatLength(kind, detail.length)}
        </span>
        {pending > 0 && <span className="key-value-pending">{pending} 处修改未提交</span>}
        {message && <span className="key-value-message">{message}</span>}
        <code className="key-value-command mono" title="读值用的命令">
          {page.sql}
        </code>
      </footer>
      <PreviewDialog state={preview} onCancel={closePreview} onSubmit={() => void submit()} language="redis" />
    </div>
  );
}
