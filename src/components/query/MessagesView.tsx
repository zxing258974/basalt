import { Ban, CircleAlert, CircleCheck, Info, LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";
import { entryDetail, entryStatus, formatMs, runSummary, type EntryStatus, type RunState } from "./runModel";

const ICONS: Record<EntryStatus, ReactNode> = {
  running: <LoaderCircle className="spin" aria-label="执行中" />,
  ok: <CircleCheck aria-label="成功" />,
  error: <CircleAlert aria-label="出错" />,
  cancelled: <Ban aria-label="已取消" />,
};

/** 「消息」页:每条语句的结果、影响行数和耗时,以及执行时服务器发来的消息(RAISE NOTICE 之类)。 */
export function MessagesView({ run }: { run: RunState }) {
  const summary = runSummary(run);
  return (
    <div className="query-messages selectable" role="log" aria-label="执行消息">
      {summary && <p className="query-messages-summary">{summary}</p>}
      <ol className="query-message-list">
        {run.entries.map((entry, index) => {
          const status = entryStatus(entry);
          return (
            <li key={index} className={`query-message is-${status}`}>
              <span className="query-message-index mono">{index + 1}</span>
              <span className="query-message-icon">{ICONS[status]}</span>
              <div className="query-message-body">
                {entry.outcome?.notice && (
                  <p className="query-message-notice">
                    <Info aria-hidden />
                    {entry.outcome.notice}
                  </p>
                )}
                <code className="query-message-sql" title={entry.sql}>
                  {entry.sql.replace(/\s+/g, " ")}
                </code>
                <p className="query-message-detail">{entryDetail(entry)}</p>
                {entry.outcome && entry.outcome.messages.length > 0 && (
                  <ul className="query-message-server" aria-label="服务器消息">
                    {entry.outcome.messages.map((message, at) => (
                      <li key={at} className={message.level === "WARNING" ? "is-warning" : undefined}>
                        <span className="query-message-level mono">{message.level}</span>
                        <span className="query-message-text">{message.text}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
              <span className="query-message-time mono">{entry.outcome ? formatMs(entry.outcome.elapsedMs) : ""}</span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
