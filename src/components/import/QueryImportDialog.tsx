import { useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { CircleCheck } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { savedQueryKeys } from "../../hooks/useSavedQueries";
import { getBackend } from "../../lib/api";
import type { QueryImportCandidate, QueryImportReport } from "../../lib/queryTypes";
import { errorMessage } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import {
  defaultQuerySelection,
  queryReportHeadline,
  querySelectionSummary,
  queryStatusLabel,
  sourcePath,
  targetPath,
  toggleQuery,
} from "./queryImportModel";

interface QueryImportDialogProps {
  scan: UseQueryResult<QueryImportCandidate[]>;
  /** 来源卡片,和导入连接共用。 */
  picker: ReactNode;
  onClose(): void;
}

const SUBTITLE = "把 Navicat 里保存的查询搬到对应连接的库下面；只新增文件，不覆盖已有的查询，也不改动 Navicat 的文件。";

/** 导入 Navicat 保存的查询:预览每条要放到哪个连接、哪个库,对不上的说明原因。 */
export function QueryImportDialog({ scan, picker, onClose }: QueryImportDialogProps) {
  const queryClient = useQueryClient();
  const candidates = scan.data ?? [];
  const [picked, setPicked] = useState<Set<string> | null>(null);
  const selection = picked ?? defaultQuerySelection(candidates);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [report, setReport] = useState<QueryImportReport | null>(null);
  // 对话框挂载时 showModal 会把焦点交给第一张来源卡片,看着像它也被选中了:
  // 等一帧再把焦点交给「导入」(没有能导入的就交给「取消」)。
  const importButton = useRef<HTMLButtonElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const importableAtOpen = useRef(selection.size > 0);
  useEffect(() => {
    const target = importableAtOpen.current ? importButton : cancelButton;
    const frame = requestAnimationFrame(() => target.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, []);

  const submit = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const ids = candidates.filter((c) => selection.has(c.id)).map((c) => c.id);
      const result = await (await getBackend()).importNavicatQueries(ids);
      await queryClient.invalidateQueries({ queryKey: savedQueryKeys.all });
      setReport(result);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  if (report) {
    return (
      <Dialog
        open
        title="导入查询"
        width={560}
        onClose={onClose}
        footer={
          <>
            <span className="footer-spacer" />
            <Button variant="primary" onClick={onClose} autoFocus>
              完成
            </Button>
          </>
        }
      >
        <QueryImportReportView report={report} />
      </Dialog>
    );
  }

  return (
    <Dialog
      open
      title="导入查询"
      subtitle={SUBTITLE}
      width={880}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <span className="import-footer-summary">{scan.data ? querySelectionSummary(candidates, selection) : ""}</span>
          <span className="footer-spacer" />
          <Button ref={cancelButton} variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button
            ref={importButton}
            variant="primary"
            busy={busy}
            disabled={!scan.data || selection.size === 0}
            onClick={() => void submit()}
          >
            {`导入 ${selection.size} 条查询`}
          </Button>
        </>
      }
    >
      <div className="import-body">
        {picker}
        {scan.error ? <p className="test-result is-error">{errorMessage(scan.error)}</p> : null}
        {scan.isPending && <p className="import-placeholder">正在读取 Navicat 保存的查询…</p>}
        {scan.data && candidates.length === 0 && <p className="import-placeholder">Navicat 里没有保存的查询。</p>}
        {candidates.length > 0 && (
          <QueryPreview
            candidates={candidates}
            selection={selection}
            onToggle={(id) => setPicked(toggleQuery(selection, candidates, id))}
          />
        )}
        {error && <p className="test-result is-error">{error}</p>}
      </div>
    </Dialog>
  );
}

interface QueryPreviewProps {
  candidates: readonly QueryImportCandidate[];
  selection: ReadonlySet<string>;
  onToggle(id: string): void;
}

function QueryPreview({ candidates, selection, onToggle }: QueryPreviewProps) {
  return (
    <div className="import-table-scroll">
      <table className="import-table">
        <thead>
          <tr>
            <th className="import-check-cell" aria-label="选择" />
            <th>查询</th>
            <th>Navicat 里的位置</th>
            <th>导入到</th>
            <th className="import-status-col">状态</th>
          </tr>
        </thead>
        <tbody>
          {candidates.map((candidate) => {
            const blocked = candidate.status.kind !== "new";
            const target = targetPath(candidate);
            const toggle = () => onToggle(candidate.id);
            return (
              <tr
                key={candidate.id}
                className={blocked ? "is-unsupported" : undefined}
                onClick={blocked ? undefined : toggle}
              >
                <td className="import-check-cell">
                  <input
                    type="checkbox"
                    aria-label={`选择「${candidate.name}」`}
                    checked={selection.has(candidate.id)}
                    disabled={blocked}
                    onChange={toggle}
                    onClick={(event) => event.stopPropagation()}
                  />
                </td>
                <td>
                  <span className="import-name">{candidate.name}</span>
                </td>
                <td className="import-source-path">{sourcePath(candidate)}</td>
                <td>
                  <div className="import-name-stack">
                    {target && <span className="import-target">{target}</span>}
                    {candidate.matchedBy === "address" && (
                      <span className="import-note">按 Navicat 里同名连接的地址匹配</span>
                    )}
                    {candidate.status.kind === "unmatched" && (
                      <span className="import-note">{candidate.status.reason}</span>
                    )}
                    {candidate.status.kind === "conflict" && (
                      <span className="import-note">这个库下已有同名、内容不同的查询，不覆盖</span>
                    )}
                  </div>
                </td>
                <td>
                  <span className={`import-status is-${candidate.status.kind}`}>
                    {queryStatusLabel(candidate.status)}
                  </span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function QueryImportReportView({ report }: { report: QueryImportReport }) {
  return (
    <div className="import-report">
      <p className="import-report-headline">
        <CircleCheck aria-hidden />
        {queryReportHeadline(report)}
      </p>
      {report.imported.length > 0 && (
        <ul className="import-report-list">
          {report.imported.map((item) => (
            <li key={`${item.target.connectionId}/${item.target.database}/${item.target.schema ?? ""}/${item.name}`}>
              <span className="import-report-name">{item.name}</span>
              <span className="import-report-meta">
                {[item.target.connectionName, item.target.database, item.target.schema].filter(Boolean).join(" / ")}
              </span>
            </li>
          ))}
        </ul>
      )}
      {report.skipped.length > 0 && (
        <>
          <h3 className="import-report-subtitle">跳过</h3>
          <ul className="import-report-list">
            {report.skipped.map((item, index) => (
              <li key={`${index}-${item.name}`}>
                <span className="import-report-name">{item.name}</span>
                <span className="import-report-meta">{item.reason}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      {report.imported.length > 0 && (
        <p className="import-hint">在对象树里展开对应的库（PostgreSQL 是 schema），「查询」分组里就能看到它们。</p>
      )}
    </div>
  );
}
