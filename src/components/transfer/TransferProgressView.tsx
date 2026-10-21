import { useEffect, useState } from "react";
import { Button } from "../ui/Button";
import { formatCount, formatElapsed, phaseLabel } from "./exportModel";
import type { JobState } from "./useTransferJob";

/** 每秒刷新一次用时。 */
function useNow(running: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [running]);
  return now;
}

interface TransferProgressViewProps {
  job: Extract<JobState, { status: "running" }>;
  /** 「行」前面的动词:已导出、已导入。 */
  verb: string;
  onCancel(): void;
}

/** 进行中:大号的行数、阶段、用时;知道总量时是进度条,不知道时是来回走的条。 */
export function TransferProgressView({ job, verb, onCancel }: TransferProgressViewProps) {
  const now = useNow(true);
  const progress = job.progress;
  const fraction = progress?.fraction ?? null;
  const rows = progress?.rows ?? 0;
  return (
    <div className="transfer-progress" role="status" aria-live="polite">
      <div className="transfer-progress-head">
        <span className="transfer-phase">
          {job.cancelling ? "正在取消" : phaseLabel(progress?.phase ?? null)}
          <span className="transfer-ellipsis" aria-hidden>
            …
          </span>
        </span>
        <span className="transfer-elapsed mono">{formatElapsed(now - job.startedAt)}</span>
      </div>
      <p className="transfer-count">
        <span className="transfer-count-value mono">{formatCount(rows)}</span>
        <span className="transfer-count-unit">行{verb}</span>
      </p>
      <div
        className={fraction === null ? "transfer-bar is-indeterminate" : "transfer-bar"}
        role="progressbar"
        aria-label="进度"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}
      >
        <span
          className="transfer-bar-fill"
          style={fraction === null ? undefined : { transform: `scaleX(${fraction})` }}
        />
      </div>
      {progress && progress.failed > 0 && (
        <p className="transfer-failed">其中 {formatCount(progress.failed)} 行失败，结束后可以查看原因。</p>
      )}
      <div className="transfer-progress-actions">
        <Button variant="secondary" disabled={job.cancelling} onClick={onCancel}>
          {job.cancelling ? "正在取消…" : "取消"}
        </Button>
      </div>
    </div>
  );
}
