import { useState } from "react";
import { getBackend } from "../../lib/api";
import type { ExportOutcome } from "../../lib/transferTypes";
import { errorMessage, isCommandError, type ConnectionView, type TableRef } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ExportDoneBody, ExportDoneFooter } from "../transfer/ExportDone";
import { formatBytes, formatCount, safeFileName } from "../transfer/exportModel";
import { TransferProgressView } from "../transfer/TransferProgressView";
import { useTransferJob } from "../transfer/useTransferJob";
import { ChoiceList, type Choice } from "./ChoiceList";
import { DUMP_NOTES, qualifiedName } from "./tableOpsModel";

interface DumpTableDialogProps {
  connection: ConnectionView;
  table: TableRef;
  onClose(): void;
}

const CHOICES: Choice<boolean>[] = [
  { value: true, label: "结构和数据", detail: "建表语句，加上全部行的 INSERT 语句。" },
  { value: false, label: "仅结构", detail: "只有建表语句，连同索引和注释。" },
];

const SQL_FILES = { name: "SQL 文件", extensions: ["sql"] };

/** 取消是平常事,出错才用警示色。 */
type Message = { tone: "error" | "note"; text: string } | null;

type View = { kind: "options"; message: Message } | { kind: "done"; outcome: ExportOutcome; withData: boolean };

/** 转储 SQL 文件:选内容、选保存位置,写的时候看进度、能取消。 */
export function DumpTableDialog({ connection, table, onClose }: DumpTableDialogProps) {
  const [withData, setWithData] = useState(true);
  const [view, setView] = useState<View>({ kind: "options", message: null });
  const { job, run, cancel } = useTransferJob();
  const running = job.status === "running";

  const start = async () => {
    try {
      const backend = await getBackend();
      const path = await backend.pickSavePath(`${safeFileName(table.table)}.sql`, SQL_FILES);
      if (!path) return;
      const request = { connectionId: connection.id, table, withData, path };
      const outcome = await run((b, jobId) => b.dumpTable(jobId, request));
      setView({ kind: "done", outcome, withData });
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
        title="转储完成"
        width={520}
        onClose={onClose}
        footer={<ExportDoneFooter path={outcome.path} onClose={onClose} />}
      >
        <ExportDoneBody
          outcome={outcome}
          summary={
            view.withData ? (
              <>
                已转储表结构和 <span className="mono">{formatCount(outcome.rows)}</span> 行数据，文件{" "}
                {formatBytes(outcome.bytes)}
              </>
            ) : (
              <>已转储表结构，文件 {formatBytes(outcome.bytes)}</>
            )
          }
        />
      </Dialog>
    );
  }

  const { message } = view;
  return (
    <Dialog
      open
      title="转储 SQL 文件"
      subtitle={`${connection.name} · ${qualifiedName(table)}`}
      width={520}
      // 写文件时按 Esc 不关:后端还在写,关掉就看不到结果了。要停请点取消。
      onClose={running ? () => undefined : onClose}
      footer={
        running ? undefined : (
          <>
            <span className="footer-spacer" />
            <Button variant="ghost" onClick={onClose}>
              取消
            </Button>
            <Button variant="primary" autoFocus onClick={() => void start()}>
              转储…
            </Button>
          </>
        )
      }
    >
      {running ? (
        <TransferProgressView job={job} verb="已导出" onCancel={() => void cancel()} />
      ) : (
        <div className="table-ops-form">
          <ChoiceList label="转储内容" choices={CHOICES} value={withData} onChange={setWithData} />
          <p className="dialog-text is-muted">{DUMP_NOTES[connection.driver]}</p>
          {message?.tone === "error" && (
            <p className="transfer-error" role="alert">
              {message.text}
            </p>
          )}
          {message?.tone === "note" && (
            <p className="transfer-cancelled" role="status">
              {message.text}
            </p>
          )}
        </div>
      )}
    </Dialog>
  );
}
