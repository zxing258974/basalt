import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { tableDataKeys } from "../../hooks/useTableData";
import { getBackend } from "../../lib/api";
import { hasPrimaryModifier } from "../../lib/platform";
import type { ResultSet, ResultSort } from "../../lib/queryTypes";
import { errorMessage, type RowChange, type TableRef } from "../../lib/types";
import { DataGrid } from "../data/DataGrid";
import * as editing from "../data/editState";
import { nextSort } from "../data/GridHeader";
import { fitSelection, gridRows, type GridSelection } from "../data/gridModel";
import { PreviewDialog } from "../data/PreviewDialog";
import { useGridEdits } from "../data/useGridEdits";
import { EmptyState } from "../ui/EmptyState";
import {
  editPlan,
  joinedChangeProblem,
  sameTable,
  sourceTables,
  tableLabel,
  toTableChanges,
  trustedSources,
  type EditPlan,
} from "./resultEditing";
import { ResultFooter } from "./ResultFooter";
import { resultPage, sortKey, toResultSort } from "./resultModel";
import type { SqlDialect } from "./splitStatements";
import { useResultRows } from "./useResultRows";

interface ResultGridProps {
  editorId: string;
  connectionId: string;
  dialect: SqlDialect;
  /** 产生这个结果的语句:判断能不能改。 */
  sql: string;
  set: ResultSet;
  label: string;
  /** 正在执行别的语句、连接关着:先不让改。 */
  frozen: boolean;
  /** 上一个结果提交修改后留下的说明(提交后重新执行,换成了这个结果)。 */
  notice: string | null;
  onPendingChange(count: number): void;
  /** 修改提交了:重新执行这条语句,换上新结果。 */
  onCommitted(message: string): Promise<void>;
  /** 点了列头排序:导出时照这个顺序。 */
  onSortChange(sort: ResultSort | null): void;
}

const resultTableKey = (connectionId: string, table: TableRef | null) => ["resultTable", connectionId, table] as const;

interface EditChoice {
  plan: EditPlan;
  /** 连接了几张表、不止一张能改时可以选改哪张;否则为空。 */
  candidates: TableRef[];
  table: TableRef | null;
}

/** 能改的条件:语句和列来源先看一遍,再读要改的那张表的结构核对主键。 */
function useEditPlan(
  connectionId: string,
  dialect: SqlDialect,
  sql: string,
  set: ResultSet,
  chosen: TableRef | null,
): EditChoice {
  const check = useMemo(() => sourceTables(sql, set, dialect), [dialect, set, sql]);
  const candidates = "tables" in check ? check.tables : [];
  const table = candidates.find((t) => chosen !== null && sameTable(t, chosen)) ?? candidates[0] ?? null;
  const columns = useQuery({
    queryKey: resultTableKey(connectionId, table),
    queryFn: async () => (await getBackend()).listColumns(connectionId, table!),
    enabled: table !== null,
    staleTime: 30_000,
  });
  const choice = (plan: EditPlan): EditChoice => ({ plan, candidates: candidates.length > 1 ? candidates : [], table });
  if ("reason" in check || table === null) {
    return choice({ kind: "readOnly", reason: "reason" in check ? check.reason : "没有能改的表。" });
  }
  if (columns.data) return choice(editPlan(set, table, columns.data, check.joined));
  if (columns.error) {
    return choice({
      kind: "readOnly",
      reason: `读不到 ${tableLabel(table)} 的表结构，不能在这里改：${errorMessage(columns.error)}`,
    });
  }
  return choice({ kind: "readOnly", reason: "正在读取表结构…" });
}

/**
 * 交给后端前换成表里的列名。改到不是这张表的列(换表之前留下的修改)就拒绝,
 * 连接了几张表时再核对一遍(见 joinedChangeProblem)。
 */
function tableChanges(plan: EditPlan & { kind: "editable" }, rows: RowChange[]): RowChange[] {
  const names = rows.flatMap((row) => [
    ...Object.keys(row.kind === "insert" ? {} : row.key),
    ...Object.keys(row.kind === "delete" ? {} : row.values),
  ]);
  const stray = names.find((name) => !(name in plan.tableColumn));
  if (stray !== undefined) throw new Error(`「${stray}」不是 ${tableLabel(plan.table)} 的列，不能提交。`);
  const changes = toTableChanges(rows, plan.tableColumn);
  const problem = plan.rowEdits ? null : joinedChangeProblem(changes, plan.table);
  if (problem) throw new Error(problem);
  return changes;
}

/** 一个结果集:往下滚时分批取行,点列头由后端排序;来自一张带主键的表时能直接改。 */
export function ResultGrid(props: ResultGridProps) {
  const { editorId, connectionId, set } = props;
  const queryClient = useQueryClient();
  const result = useResultRows(editorId, set);
  // 子查询算出来的列不算表里的列(见 trustedSources):表头、能不能改都按它。
  const trusted = useMemo(() => trustedSources(props.sql, set, props.dialect), [props.dialect, props.sql, set]);
  const [chosen, setChosen] = useState<TableRef | null>(null);
  const { plan, candidates, table: editTable } = useEditPlan(connectionId, props.dialect, props.sql, trusted, chosen);
  const page = useMemo(
    () =>
      resultPage(
        trusted,
        result.rows,
        plan.kind === "editable" ? plan.columns : null,
        plan.kind === "editable" ? null : plan.reason,
      ),
    [plan, result.rows, trusted],
  );
  const target = plan.kind === "editable" ? plan : null;

  // 修改按行号记:整批行换了(换排序)、换了要改的表就作废,绝不套到别的行、别的表上。
  const tableKey = editTable ? [editTable.database, editTable.schema ?? "", editTable.table].join(".") : "";
  const changes = useGridEdits(page, `${set.id}:${result.version}:${tableKey}`, {
    preview: async (rows) =>
      (await getBackend()).previewTableChanges(connectionId, target!.table, tableChanges(target!, rows)),
    apply: async (rows, statements) =>
      (await getBackend()).applyTableChanges(connectionId, target!.table, tableChanges(target!, rows), statements),
    refresh: async (outcome) => {
      void queryClient.invalidateQueries({ queryKey: tableDataKeys.table(connectionId, target!.table) });
      await props.onCommitted(`已提交 ${outcome.statements} 条语句，影响 ${outcome.affectedRows} 行。`);
    },
  });
  const { edits, pending, actions } = changes;
  // 换了排序、新顺序的行还没到时手上的行马上要被换掉,不能改。
  const editable = target !== null && !props.frozen && !result.stale;

  const [selection, setSelection] = useState<GridSelection | null>(null);
  const rows = useMemo(() => gridRows(page, edits), [page, edits]);
  const fitted = fitSelection(selection, rows.length, page.columns.length);
  if (fitted !== selection) setSelection(fitted);
  const selectedRow = fitted ? rows[fitted.row] : undefined;

  const { onPendingChange, onSortChange } = props;
  useEffect(() => onPendingChange(pending), [onPendingChange, pending]);
  // 新顺序的行到了才算数:排序请求失败时导出不该按表头上那个没生效的排序。
  useEffect(() => {
    if (!result.stale) onSortChange(toResultSort(result.sort));
  }, [onSortChange, result.sort, result.stale]);
  useEffect(() => () => onPendingChange(0), [onPendingChange]);

  // ⌘S 在结果表格里按下、有没提交的修改(或正在改一格,这一格马上会提交)时是提交;
  // 别的时候留给查询标签页保存 SQL。等编辑框先提交那一格,下一轮再读最新的修改。
  const onKeyDown = (event: KeyboardEvent) => {
    if (!hasPrimaryModifier(event) || event.shiftKey || event.altKey || event.key.toLowerCase() !== "s") return;
    const inCellEditor = event.target instanceof HTMLElement && event.target.classList.contains("cell-editor");
    if (pending === 0 && !inCellEditor) return;
    event.preventDefault();
    window.setTimeout(() => void changes.openPreview(), 0);
  };

  const addRow = () => {
    changes.setEdits((current) => editing.addRow(current));
    const first = page.columns.findIndex((c) => !c.isGenerated && !c.hasDefault);
    setSelection({ row: rows.length, col: Math.max(first, 0) });
    changes.requestFocus();
  };

  if (set.columns.length === 0) return <EmptyState title="这个结果集没有列" />;
  return (
    <div className="result-grid" onKeyDown={onKeyDown}>
      <DataGrid
        label={props.label}
        page={page}
        edits={edits}
        rowOffset={0}
        editable={editable}
        rowEdits={target?.rowEdits ?? true}
        selection={fitted}
        onSelectionChange={setSelection}
        sort={result.sort}
        sortDisabled={pending > 0}
        sortKeyOf={(column, index) => sortKey(index, column.name)}
        onSort={(key) => result.changeSort(nextSort(result.sort, key))}
        copyOnlyMenu={!editable}
        actions={actions}
        focusRequest={changes.focusRequest}
        scrollKey={result.sort ? `${result.sort.column}:${result.sort.descending}` : ""}
        // 有新增的行时先不往下取:新行排在已取到的行后面,取来的行会插到它前面,选区和编辑框就对不上了。
        onNearEnd={result.hasMore && edits.inserted.length === 0 ? result.loadMore : undefined}
      />
      <ResultFooter
        loaded={result.rows.length}
        total={set.totalRows}
        truncated={set.truncated}
        loading={result.loading}
        error={result.error}
        onRetry={result.retry}
        readOnlyReason={plan.kind === "editable" ? null : plan.reason}
        message={changes.message ?? props.notice}
        editTables={candidates}
        editTable={editTable}
        onEditTableChange={(table) => {
          if (pending === 0) setChosen(table);
        }}
        editable={editable}
        rowEdits={target?.rowEdits ?? true}
        selectedDeleted={selectedRow?.status === "deleted"}
        canDelete={selectedRow !== undefined}
        onAddRow={addRow}
        onToggleDelete={() => {
          if (selectedRow?.status === "deleted") actions.restoreRow(selectedRow);
          else if (selectedRow) actions.deleteRow(selectedRow);
        }}
        pending={pending}
        onRevert={() => changes.setEdits(editing.EMPTY_EDITS)}
        onCommit={() => void changes.openPreview()}
      />
      <PreviewDialog state={changes.preview} onCancel={changes.closePreview} onSubmit={() => void changes.submit()} />
    </div>
  );
}
