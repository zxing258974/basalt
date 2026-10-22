// 表格里的未提交修改与「预览 → 提交」:表数据页和可改的查询结果共用。
// 修改按行号记,所以数据换了(resetKey 变了)就丢掉旧修改,绝不套到别的行上。
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { errorMessage, type ApplyOutcome, type RowChange, type TablePage } from "../../lib/types";
import type { GridActions } from "./DataGrid";
import * as editing from "./editState";
import type { PreviewState } from "./PreviewDialog";

export interface GridEditOps {
  /** 生成要执行的语句,不执行。 */
  preview(changes: RowChange[]): Promise<string[]>;
  /** 在一个事务里执行预览过的语句。 */
  apply(changes: RowChange[], statements: string[]): Promise<ApplyOutcome>;
  /** 提交成功后重读数据;等它完成再关对话框、清修改,免得先闪回旧值。失败也照样收尾:数据库里已经提交了。 */
  refresh(outcome: ApplyOutcome): Promise<void>;
}

export function useGridEdits(page: TablePage | undefined, resetKey: unknown, ops: GridEditOps) {
  const [edits, setEdits] = useState(editing.EMPTY_EDITS);
  const [preview, setPreview] = useState<PreviewState>({ phase: "closed" });
  const [message, setMessage] = useState<string | null>(null);
  const [focusRequest, setFocusRequest] = useState(0);
  const pending = editing.pendingCount(edits);

  const [editsFor, setEditsFor] = useState(resetKey);
  if (editsFor !== resetKey) {
    setEditsFor(resetKey);
    setEdits(editing.EMPTY_EDITS);
  }

  // ⌘S 可能在单元格编辑框里按下:编辑框先提交那一格,下一轮再读最新的修改。
  const latest = useRef({ page, edits, preview, ops });
  useLayoutEffect(() => {
    latest.current = { page, edits, preview, ops };
  });

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
          row.target.kind === "new"
            ? editing.removeNewRow(current, row.target.id)
            : editing.deleteRow(current, row.target.index),
        ),
      restoreRow: (row) => {
        const { target } = row;
        if (target.kind === "existing") setEdits((current) => editing.restoreRow(current, target.index));
      },
    }),
    [page],
  );

  // 每打开或关闭一次预览加一:生成 SQL 期间用户取消了,迟到的结果不能再把对话框弹回来。
  const previewToken = useRef(0);
  const closePreview = useCallback(() => {
    previewToken.current += 1;
    setPreview({ phase: "closed" });
    setFocusRequest((n) => n + 1);
  }, []);

  const openPreview = useCallback(async () => {
    const { page: current, edits: currentEdits, preview: shown, ops: currentOps } = latest.current;
    if (!current || shown.phase !== "closed" || editing.pendingCount(currentEdits) === 0) return;
    const changes = editing.buildChanges(currentEdits, current);
    const token = ++previewToken.current;
    setPreview({ phase: "loading" });
    try {
      const statements = await currentOps.preview(changes);
      if (token === previewToken.current)
        setPreview({ phase: "ready", statements, changes, applying: false, error: null });
    } catch (error) {
      if (token === previewToken.current) setPreview({ phase: "invalid", message: errorMessage(error) });
    }
  }, []);

  const submit = useCallback(async () => {
    const { preview: shown, ops: currentOps } = latest.current;
    if (shown.phase !== "ready" || shown.applying) return;
    setPreview({ ...shown, applying: true, error: null });
    try {
      const outcome = await currentOps.apply(shown.changes, shown.statements);
      await currentOps.refresh(outcome).catch((error: unknown) => console.warn("提交后重读数据失败", error));
      setEdits(editing.EMPTY_EDITS);
      closePreview();
      setMessage(`已提交 ${outcome.statements} 条语句，影响 ${outcome.affectedRows} 行。`);
    } catch (error) {
      setPreview({ ...shown, applying: false, error: errorMessage(error) });
    }
  }, [closePreview]);

  return {
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
    focusRequest,
    requestFocus: useCallback(() => setFocusRequest((n) => n + 1), []),
  };
}
