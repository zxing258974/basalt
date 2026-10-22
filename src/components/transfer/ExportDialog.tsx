import { useState } from "react";
import { getBackend } from "../../lib/api";
import type { BinaryEncoding, ExportOutcome } from "../../lib/transferTypes";
import { errorMessage, isCommandError } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ExportDoneBody, ExportDoneFooter } from "./ExportDone";
import { ExportOptionsForm } from "./ExportOptionsForm";
import {
  DEFAULT_CSV,
  DEFAULT_ROWS_PER_STATEMENT,
  buildFormat,
  buildSource,
  defaultRange,
  extensionOf,
  fileFilter,
  formatBytes,
  formatCount,
  querySummary,
  safeFileName,
  type ExportTarget,
  type FormatChoices,
  type RangeChoice,
} from "./exportModel";
import { TransferProgressView } from "./TransferProgressView";
import { useTransferJob } from "./useTransferJob";

/** 选项页下方的一行话:取消是平常事,出错才用警示色。 */
type Message = { tone: "error" | "note"; text: string } | null;

type View = { kind: "options"; message: Message } | { kind: "done"; outcome: ExportOutcome };

export function ExportDialog({ target, onClose }: { target: ExportTarget; onClose(): void }) {
  const [choices, setChoices] = useState<FormatChoices>({
    kind: "csv",
    csv: DEFAULT_CSV,
    jsonLines: false,
    sqlTable: target.baseName,
    rowsPerStatement: DEFAULT_ROWS_PER_STATEMENT,
  });
  const [binary, setBinary] = useState<BinaryEncoding>("hex");
  const [range, setRange] = useState<RangeChoice>(() => defaultRange(target));
  const [view, setView] = useState<View>({ kind: "options", message: null });
  const { job, run, cancel } = useTransferJob();
  const running = job.status === "running";

  const start = async () => {
    const format = buildFormat(choices);
    if (format.kind === "sql" && !format.table) {
      setView({ kind: "options", message: { tone: "error", text: "请填写 INSERT 语句里的表名。" } });
      return;
    }
    try {
      const backend = await getBackend();
      const path = await backend.pickSavePath(
        `${safeFileName(target.baseName)}.${extensionOf(format)}`,
        fileFilter(format),
      );
      if (!path) return;
      const request = {
        connectionId: target.connectionId,
        source: buildSource(target, range),
        options: { format, binary },
        path,
      };
      const outcome = await run((b, jobId) => b.exportData(jobId, request));
      setView({ kind: "done", outcome });
    } catch (error) {
      const cancelled = isCommandError(error) && error.code === "cancelled";
      setView({
        kind: "options",
        message: cancelled
          ? { tone: "note", text: "已取消，没有生成文件。" }
          : { tone: "error", text: errorMessage(error) },
      });
    }
  };

  if (view.kind === "done") {
    const { outcome } = view;
    return (
      <Dialog
        open
        title="导出完成"
        width={520}
        onClose={onClose}
        footer={<ExportDoneFooter path={outcome.path} onClose={onClose} />}
      >
        <ExportDoneBody
          outcome={outcome}
          summary={
            <>
              已导出 <span className="mono">{formatCount(outcome.rows)}</span> 行，文件 {formatBytes(outcome.bytes)}
            </>
          }
        />
      </Dialog>
    );
  }

  const { scope } = target;
  return (
    <Dialog
      open
      title="导出数据"
      subtitle={target.title}
      width={620}
      // 导出进行中按 Esc 不关:后端还在写,关掉就看不到结果了。要停请点取消。
      onClose={running ? () => undefined : onClose}
      footer={
        running ? undefined : (
          <>
            <span className="footer-spacer" />
            <Button variant="ghost" onClick={onClose}>
              取消
            </Button>
            <Button variant="primary" autoFocus onClick={() => void start()}>
              导出…
            </Button>
          </>
        )
      }
    >
      {running ? (
        <TransferProgressView job={job} verb="已导出" onCancel={() => void cancel()} />
      ) : (
        <div className="transfer-body">
          <section className="transfer-section" aria-label="范围">
            <h3 className="transfer-section-title">范围</h3>
            {scope.kind === "table" ? (
              <div className="transfer-range" role="radiogroup" aria-label="范围">
                <RangeOption checked={range === "all"} onSelect={() => setRange("all")} label="整张表" />
                <RangeOption
                  checked={range === "filtered"}
                  disabled={!scope.filter}
                  onSelect={() => setRange("filtered")}
                  label="当前筛选条件下的结果"
                  detail={scope.filter ?? "表数据页里没有筛选条件"}
                />
              </div>
            ) : (
              <p className="transfer-note">{querySummary(scope.set, scope.sort)}</p>
            )}
          </section>
          <section className="transfer-section" aria-label="格式">
            <h3 className="transfer-section-title">格式</h3>
            <ExportOptionsForm choices={choices} onChange={setChoices} binary={binary} onBinaryChange={setBinary} />
          </section>
          {view.message?.tone === "error" && (
            <p className="transfer-error" role="alert">
              {view.message.text}
            </p>
          )}
          {view.message?.tone === "note" && (
            <p className="transfer-cancelled" role="status">
              {view.message.text}
            </p>
          )}
        </div>
      )}
    </Dialog>
  );
}

function RangeOption(props: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  detail?: string;
  onSelect(): void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.checked}
      disabled={props.disabled}
      className={props.checked ? "transfer-range-option is-checked" : "transfer-range-option"}
      onClick={props.onSelect}
    >
      <span className="transfer-radio" aria-hidden />
      <span className="transfer-range-label">{props.label}</span>
      {props.detail && <code className="transfer-range-detail">{props.detail}</code>}
    </button>
  );
}
