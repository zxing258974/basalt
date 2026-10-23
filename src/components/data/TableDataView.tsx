import { useQueryClient } from "@tanstack/react-query";
import { Lock } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { usePublishCellPeek } from "../../hooks/usePublishCellPeek";
import { tableDataKeys, useTablePage } from "../../hooks/useTableData";
import { getBackend } from "../../lib/api";
import { errorMessage, type PageRequest } from "../../lib/types";
import type { TableDataTab } from "../../stores/tabs";
import { useTabs } from "../../stores/tabs";
import { useTransfer } from "../../stores/transfer";
import { DriverBadge } from "../connection/DriverBadge";
import { peekCell } from "../inspector/cellPeekModel";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import { DataFooter } from "./DataFooter";
import { DataGrid } from "./DataGrid";
import { DataToolbar } from "./DataToolbar";
import * as editing from "./editState";
import { nextSort } from "./GridHeader";
import { fitSelection, gridRows, type GridSelection } from "./gridModel";
import { PreviewDialog } from "./PreviewDialog";
import { useGridEdits } from "./useGridEdits";
import { hasPrimaryModifier } from "../../lib/platform";

const DEFAULT_LIMIT = 1000;

export function TableDataView({ tab, active }: { tab: TableDataTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const isOpen = Boolean(connection?.isOpen);
  const { openWithFeedback } = useConnectionActions();
  const queryClient = useQueryClient();

  const [request, setRequest] = useState<PageRequest>({ offset: 0, limit: DEFAULT_LIMIT, sort: null, filter: null });
  const [filterDraft, setFilterDraft] = useState("");
  const [selection, setSelection] = useState<GridSelection | null>(null);

  const query = useTablePage(tab.connectionId, tab.table, request, isOpen);
  const page = query.data;
  // 修改按本页行号记录。页面数据换了(提交后重读)就丢掉旧修改,绝不套到别的行上。
  const {
    edits,
    setEdits,
    pending,
    actions,
    preview,
    openPreview,
    closePreview,
    submit,
    message,
    setMessage,
    ...grid
  } = useGridEdits(page, page, {
    preview: async (changes) => (await getBackend()).previewTableChanges(tab.connectionId, tab.table, changes),
    apply: async (changes, statements) =>
      (await getBackend()).applyTableChanges(tab.connectionId, tab.table, changes, statements),
    refresh: () => queryClient.invalidateQueries({ queryKey: tableDataKeys.table(tab.connectionId, tab.table) }),
  });
  const locked = pending > 0;
  const editable = Boolean(page && !page.readOnlyReason && isOpen);
  const rows = useMemo(() => (page ? gridRows(page, edits) : []), [page, edits]);
  // 行数变少(撤销新行、提交删除后重读)时选区收进还在的行,免得按键、复制、删除行指向不存在的行。
  const fitted = fitSelection(selection, rows.length, page?.columns.length ?? 0);
  if (fitted !== selection) setSelection(fitted);
  const selectedRow = fitted ? rows[fitted.row] : undefined;
  // 选中的那一格报给信息面板的「值」页。连接关了表格就不显示,也不报。
  const showsGrid = Boolean(connection && isOpen && page);
  const peek = useMemo(
    () =>
      page && showsGrid ? peekCell(page, edits, rows, fitted, { source: tab.title, rowOffset: request.offset }) : null,
    [edits, fitted, page, request.offset, rows, showsGrid, tab.title],
  );
  usePublishCellPeek(peek);

  // 告诉标签栏这页有多少行没提交:关闭标签页或连接前要确认。
  const setPending = useTabs((s) => s.setPending);
  useEffect(() => {
    setPending(tab.id, pending);
  }, [setPending, tab.id, pending]);
  useEffect(() => () => setPending(tab.id, 0), [setPending, tab.id]);

  const navigate = (patch: Partial<PageRequest>) => {
    if (locked) return;
    setSelection(null);
    setMessage(null);
    setRequest((current) => ({ ...current, ...patch }));
  };

  const addRow = () => {
    if (!page) return;
    setEdits((current) => editing.addRow(current));
    // 选中新行第一个需要手填的格子(跳过自增、默认值和生成列)。
    const first = page.columns.findIndex((c) => !c.isGenerated && !c.hasDefault);
    setSelection({ row: rows.length, col: Math.max(first, 0) });
    grid.requestFocus();
  };

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

  const location = tab.table.schema ? `${tab.table.database}.${tab.table.schema}` : tab.table.database;
  const keys = page?.columns.filter((c) => c.isPrimaryKey).map((c) => c.name) ?? [];

  return (
    <section className="table-data" aria-label={`${tab.title} 的数据`}>
      <header className="data-header">
        <div className="data-heading">
          {connection && <DriverBadge driver={connection.driver} />}
          <span className="data-crumb">{connection?.name ?? "已关闭的连接"}</span>
          <span className="data-crumb-sep">/</span>
          <span className="data-crumb">{location}</span>
          <span className="data-crumb-sep">/</span>
          <h1 className="data-title selectable">{tab.title}</h1>
          {page && (
            <span className="data-summary">
              {page.columns.length} 列 · {keys.length ? `主键 ${keys.join("、")}` : "无主键"}
            </span>
          )}
        </div>
        <DataToolbar
          filterDraft={filterDraft}
          appliedFilter={request.filter}
          onFilterDraftChange={setFilterDraft}
          onApplyFilter={() => navigate({ filter: filterDraft.trim() || null, offset: 0 })}
          onClearFilter={() => {
            setFilterDraft("");
            navigate({ filter: null, offset: 0 });
          }}
          locked={locked}
          refreshing={query.isFetching}
          onRefresh={() => {
            setMessage(null);
            void query.refetch();
          }}
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
          onExport={() =>
            useTransfer.getState().openExport({
              connectionId: tab.connectionId,
              driver: connection?.driver ?? "mysql",
              title: `${location}.${tab.table.table}`,
              baseName: tab.table.table,
              scope: { kind: "table", table: tab.table, filter: request.filter, sort: request.sort },
            })
          }
        />
      </header>

      {page?.readOnlyReason && (
        <p className="data-notice">
          <Lock aria-hidden />
          {page.readOnlyReason}
        </p>
      )}

      {!connection ? (
        <EmptyState title="这个连接已经不存在" />
      ) : !isOpen ? (
        <EmptyState
          title="连接已关闭"
          action={
            <Button variant="primary" onClick={() => void openWithFeedback(connection)}>
              重新打开
            </Button>
          }
        />
      ) : query.error && !page ? (
        <EmptyState
          title="读取数据失败"
          detail={errorMessage(query.error)}
          tone="error"
          action={
            request.filter ? (
              <Button
                onClick={() => {
                  setFilterDraft("");
                  navigate({ filter: null, offset: 0 });
                }}
              >
                清除筛选
              </Button>
            ) : (
              <Button onClick={() => void query.refetch()}>重试</Button>
            )
          }
        />
      ) : !page ? (
        <EmptyState title="正在读取数据…" />
      ) : (
        <DataGrid
          label={`${tab.title} 的数据`}
          page={page}
          edits={edits}
          rowOffset={request.offset}
          editable={editable}
          selection={fitted}
          onSelectionChange={setSelection}
          sort={request.sort}
          sortDisabled={locked}
          onSort={(column) => navigate({ sort: nextSort(request.sort, column), offset: 0 })}
          actions={actions}
          focusRequest={grid.focusRequest}
          scrollKey={JSON.stringify(request)}
        />
      )}

      <DataFooter
        offset={request.offset}
        rowCount={page?.rows.length ?? 0}
        limit={request.limit}
        hasMore={page?.hasMore ?? false}
        locked={locked}
        onPrevious={() => navigate({ offset: Math.max(0, request.offset - request.limit) })}
        onNext={() => navigate({ offset: request.offset + request.limit })}
        onLimitChange={(limit) => navigate({ limit, offset: 0 })}
        pending={pending}
        message={message}
        sql={page?.sql ?? null}
      />

      <PreviewDialog state={preview} onCancel={closePreview} onSubmit={() => void submit()} />
    </section>
  );
}
