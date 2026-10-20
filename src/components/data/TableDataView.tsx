import { useQueryClient } from "@tanstack/react-query";
import { Lock } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { tableDataKeys, useTablePage } from "../../hooks/useTableData";
import { getBackend } from "../../lib/api";
import { errorMessage, type PageRequest } from "../../lib/types";
import type { TableDataTab } from "../../stores/tabs";
import { useTabs } from "../../stores/tabs";
import { DriverBadge } from "../connection/DriverBadge";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import { DataFooter } from "./DataFooter";
import { DataGrid, type GridActions, type GridSelection } from "./DataGrid";
import { DataToolbar } from "./DataToolbar";
import * as editing from "./editState";
import { nextSort } from "./GridHeader";
import { gridRows } from "./gridModel";
import { PreviewDialog, type PreviewState } from "./PreviewDialog";

const DEFAULT_LIMIT = 1000;

export function TableDataView({ tab, active }: { tab: TableDataTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const isOpen = Boolean(connection?.isOpen);
  const { openWithFeedback } = useConnectionActions();
  const queryClient = useQueryClient();

  const [request, setRequest] = useState<PageRequest>({ offset: 0, limit: DEFAULT_LIMIT, sort: null, filter: null });
  const [filterDraft, setFilterDraft] = useState("");
  const [edits, setEdits] = useState(editing.EMPTY_EDITS);
  const [selection, setSelection] = useState<GridSelection | null>(null);
  const [preview, setPreview] = useState<PreviewState>({ phase: "closed" });
  const [message, setMessage] = useState<string | null>(null);
  const [focusRequest, setFocusRequest] = useState(0);

  const query = useTablePage(tab.connectionId, tab.table, request, isOpen);
  const page = query.data;
  const pending = editing.pendingCount(edits);
  const locked = pending > 0;
  const editable = Boolean(page && !page.readOnlyReason && isOpen);
  const rows = useMemo(() => (page ? gridRows(page, edits) : []), [page, edits]);
  const selectedRow = selection ? rows[selection.row] : undefined;

  // 告诉标签栏这页有多少行没提交:关闭标签页或连接前要确认。
  const setPending = useTabs((s) => s.setPending);
  useEffect(() => {
    setPending(tab.id, pending);
  }, [setPending, tab.id, pending]);
  useEffect(() => () => setPending(tab.id, 0), [setPending, tab.id]);

  // 修改按本页行号记录。页面数据换了(提交后重读)就丢掉旧修改,绝不套到别的行上。
  const [editsFor, setEditsFor] = useState(page);
  if (editsFor !== page) {
    setEditsFor(page);
    setEdits(editing.EMPTY_EDITS);
  }

  const navigate = (patch: Partial<PageRequest>) => {
    if (locked) return;
    setSelection(null);
    setMessage(null);
    setRequest((current) => ({ ...current, ...patch }));
  };

  const actions: GridActions = useMemo(
    () => ({
      setCell: (target, column, value) => {
        if (!page) return;
        setMessage(null);
        setEdits((current) => editing.setCell(current, page, target, column, value));
      },
      revertCell: (target, column) => setEdits((current) => editing.revertCell(current, target, column)),
      deleteRow: (row) =>
        setEdits((current) =>
          row.target.kind === "new" ? editing.removeNewRow(current, row.target.id) : editing.deleteRow(current, row.target.index),
        ),
      restoreRow: (row) => {
        const { target } = row;
        if (target.kind === "existing") setEdits((current) => editing.restoreRow(current, target.index));
      },
    }),
    [page],
  );

  const addRow = () => {
    if (!page) return;
    setEdits((current) => editing.addRow(current));
    // 选中新行第一个需要手填的格子(跳过自增、默认值和生成列)。
    const first = page.columns.findIndex((c) => !c.isGenerated && !c.hasDefault);
    setSelection({ row: rows.length, col: Math.max(first, 0) });
    setFocusRequest((n) => n + 1);
  };

  // ⌘S 可能在单元格编辑框里按下:编辑框先提交那一格,下一轮再读最新的修改。
  const latest = useRef({ page, edits, preview });
  useLayoutEffect(() => {
    latest.current = { page, edits, preview };
  });

  // 每打开或关闭一次预览加一:生成 SQL 期间用户取消了,迟到的结果不能再把对话框弹回来。
  const previewToken = useRef(0);
  const closePreview = useCallback(() => {
    previewToken.current += 1;
    setPreview({ phase: "closed" });
    setFocusRequest((n) => n + 1);
  }, []);

  const openPreview = useCallback(async () => {
    const { page: current, edits: currentEdits, preview: shown } = latest.current;
    if (!current || shown.phase !== "closed" || editing.pendingCount(currentEdits) === 0) return;
    const changes = editing.buildChanges(currentEdits, current);
    const token = ++previewToken.current;
    setPreview({ phase: "loading" });
    try {
      const statements = await (await getBackend()).previewTableChanges(tab.connectionId, tab.table, changes);
      if (token === previewToken.current) setPreview({ phase: "ready", statements, changes, applying: false, error: null });
    } catch (error) {
      if (token === previewToken.current) setPreview({ phase: "invalid", message: errorMessage(error) });
    }
  }, [tab.connectionId, tab.table]);

  const submit = useCallback(async () => {
    const shown = latest.current.preview;
    if (shown.phase !== "ready" || shown.applying) return;
    setPreview({ ...shown, applying: true, error: null });
    try {
      const outcome = await (await getBackend()).applyTableChanges(
        tab.connectionId,
        tab.table,
        shown.changes,
        shown.statements,
      );
      // 等重读完再关对话框、清修改:否则会先闪回旧值。重读失败也照样收尾,数据库里已经提交了。
      await queryClient.invalidateQueries({ queryKey: tableDataKeys.table(tab.connectionId, tab.table) });
      setEdits(editing.EMPTY_EDITS);
      closePreview();
      setMessage(`已提交 ${outcome.statements} 条语句，影响 ${outcome.affectedRows} 行。`);
    } catch (error) {
      setPreview({ ...shown, applying: false, error: errorMessage(error) });
    }
  }, [closePreview, queryClient, tab.connectionId, tab.table]);

  useEffect(() => {
    if (!active) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.metaKey && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "s") {
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
          selection={selection}
          onSelectionChange={setSelection}
          sort={request.sort}
          sortDisabled={locked}
          onSort={(column) => navigate({ sort: nextSort(request.sort, column), offset: 0 })}
          actions={actions}
          focusRequest={focusRequest}
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

      <PreviewDialog
        state={preview}
        onCancel={closePreview}
        onSubmit={() => void submit()}
      />
    </section>
  );
}
