// 跑一次导入或导出:起任务 id、认领它的进度事件、能取消。
import { useCallback, useEffect, useRef, useState } from "react";
import { getBackend, type Backend } from "../../lib/api";
import type { TransferProgress } from "../../lib/transferTypes";
import { errorMessage } from "../../lib/types";
import { useUi } from "../../stores/ui";

export type JobState =
  | { status: "idle" }
  | { status: "running"; jobId: string; startedAt: number; progress: TransferProgress | null; cancelling: boolean };

let counter = 0;

/** 任务 id:前端起好,命令还没返回时也能按它取消。 */
function newJobId(): string {
  counter += 1;
  const random = typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : String(Date.now());
  return `${random}-${counter}`;
}

export function useTransferJob() {
  const [job, setJob] = useState<JobState>({ status: "idle" });
  const current = useRef<string | null>(null);
  // 对话框关掉(组件卸载)时任务还在跑:取消它,免得在后台一直写。
  useEffect(
    () => () => {
      const jobId = current.current;
      if (jobId) void getBackend().then((backend) => backend.cancelTransfer(jobId));
    },
    [],
  );

  const run = useCallback(async <T>(start: (backend: Backend, jobId: string) => Promise<T>): Promise<T> => {
    const backend = await getBackend();
    const jobId = newJobId();
    current.current = jobId;
    setJob({ status: "running", jobId, startedAt: Date.now(), progress: null, cancelling: false });
    const unlisten = await backend.onTransferProgress((progress) => {
      if (progress.jobId !== jobId) return;
      setJob((state) => (state.status === "running" && state.jobId === jobId ? { ...state, progress } : state));
    });
    try {
      return await start(backend, jobId);
    } finally {
      unlisten();
      current.current = null;
      setJob({ status: "idle" });
    }
  }, []);

  const cancel = useCallback(async () => {
    const jobId = current.current;
    if (!jobId) return;
    setJob((state) => (state.status === "running" ? { ...state, cancelling: true } : state));
    try {
      await (await getBackend()).cancelTransfer(jobId);
    } catch (error) {
      useUi.getState().notify(`取消失败：${errorMessage(error)}`);
    }
  }, []);

  return { job, run, cancel };
}
