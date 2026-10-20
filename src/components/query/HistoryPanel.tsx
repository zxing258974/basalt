import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Search, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { getBackend } from "../../lib/api";
import { formatWhen } from "../../lib/format";
import type { HistoryEntry } from "../../lib/queryTypes";
import { errorMessage } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { formatMs } from "./runModel";

/** 一次最多列出多少条;再早的用搜索找。 */
const LIMIT = 200;
const SEARCH_DELAY_MS = 200;

interface HistoryPanelProps {
  connectionId: string;
  onPick(sql: string): void;
  onClose(): void;
}

function useDebounced<T>(value: T, delay: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(value), delay);
    return () => window.clearTimeout(timer);
  }, [value, delay]);
  return settled;
}

function outcomeText(entry: HistoryEntry): string {
  if (entry.status === "error") return "出错";
  if (entry.status === "cancelled") return "已取消";
  if (entry.rowCount !== null) return `${entry.rowCount} 行`;
  if (entry.affectedRows !== null) return `影响 ${entry.affectedRows} 行`;
  return "成功";
}

/** 执行历史:最新的在上面,点一条把 SQL 填进编辑器。 */
export function HistoryPanel({ connectionId, onPick, onClose }: HistoryPanelProps) {
  const queryClient = useQueryClient();
  const [text, setText] = useState("");
  const [onlyHere, setOnlyHere] = useState(true);
  const [confirming, setConfirming] = useState(false);
  const search = useDebounced(text.trim(), SEARCH_DELAY_MS);
  const scope = onlyHere ? connectionId : null;
  const history = useQuery({
    queryKey: ["history", search, scope],
    queryFn: async () => (await getBackend()).searchHistory(search, scope, LIMIT),
    staleTime: 0,
  });

  const clear = async () => {
    setConfirming(false);
    try {
      await (await getBackend()).clearHistory();
    } catch (error) {
      useUi.getState().notify(`清空执行历史失败：${errorMessage(error)}`);
    }
    await queryClient.invalidateQueries({ queryKey: ["history"] });
  };

  const entries = history.data ?? [];
  return (
    <aside className="query-history" aria-label="执行历史">
      <header className="query-history-header">
        <h2 className="query-history-title">执行历史</h2>
        <Button variant="ghost" size="sm" icon={<X />} aria-label="关闭执行历史" onClick={onClose} />
      </header>
      <div className="query-history-search">
        <Search aria-hidden />
        <input
          type="search"
          aria-label="搜索执行历史"
          placeholder="搜索 SQL、连接或库"
          value={text}
          onChange={(event) => setText(event.target.value)}
          spellCheck={false}
        />
      </div>
      <div className="query-history-filters">
        <label className="checkbox">
          <input type="checkbox" checked={onlyHere} onChange={(event) => setOnlyHere(event.target.checked)} />
          <span>只看这个连接</span>
        </label>
        {confirming ? (
          <span className="query-history-confirm">
            <Button variant="danger" size="sm" onClick={() => void clear()}>
              确定清空
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setConfirming(false)}>
              取消
            </Button>
          </span>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            icon={<Trash2 />}
            disabled={entries.length === 0}
            onClick={() => setConfirming(true)}
          >
            清空
          </Button>
        )}
      </div>
      {history.error ? (
        <p className="query-history-empty is-error">{errorMessage(history.error)}</p>
      ) : entries.length === 0 ? (
        <p className="query-history-empty">
          {history.isPending ? "读取中…" : search ? "没有找到" : "还没有执行过语句"}
        </p>
      ) : (
        <ul className="query-history-list">
          {entries.map((entry) => (
            <li key={entry.id}>
              <button
                className={`query-history-item is-${entry.status}`}
                title="点一下填进编辑器"
                onClick={() => onPick(entry.sql)}
              >
                <span className="query-history-meta">
                  <span className="query-history-dot" aria-hidden />
                  <span className="mono">{formatWhen(entry.executedAt)}</span>
                  <span className="query-history-where">
                    {entry.connectionName}
                    {entry.database ? ` / ${entry.database}` : ""}
                  </span>
                  <span className="query-history-outcome">
                    {outcomeText(entry)} · {formatMs(entry.elapsedMs)}
                  </span>
                </span>
                <code className="query-history-sql">{entry.sql}</code>
              </button>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}
