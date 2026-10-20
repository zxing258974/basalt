import { Table2, TriangleAlert } from "lucide-react";
import { MessagesView } from "./MessagesView";
import { ResultGrid } from "./ResultGrid";
import { resultTabs, type RunState } from "./runModel";

export const MESSAGES = "messages";

interface ResultPanelProps {
  title: string;
  run: RunState;
  /** 选中的标签;null 或找不到时显示第一个。 */
  active: string | null;
  onActiveChange(key: string): void;
}

/** 编辑器下面的结果区:每个结果集一个标签,最后是「消息」。 */
export function ResultPanel({ title, run, active, onActiveChange }: ResultPanelProps) {
  const tabs = resultTabs(run.entries);
  if (run.entries.length === 0) {
    return (
      <section className="query-results is-empty" aria-label="执行结果">
        <p className="query-results-placeholder">
          {run.running ? "正在执行…" : "执行的结果会显示在这里。⌘↩ 执行光标所在的语句，⇧⌘↩ 全部执行。"}
        </p>
      </section>
    );
  }
  const shown = tabs.find((t) => t.key === active) ?? (active === MESSAGES ? null : tabs[0]) ?? null;
  const failed = run.entries.some((e) => e.failure !== null || (e.outcome && e.outcome.status !== "ok"));
  return (
    <section className="query-results" aria-label="执行结果">
      <div className="query-result-tabs" role="tablist" aria-label="结果">
        {tabs.map((tab) => (
          <button
            key={tab.key}
            role="tab"
            aria-selected={shown?.key === tab.key}
            className="query-result-tab"
            title={tab.sql}
            onClick={() => onActiveChange(tab.key)}
          >
            <Table2 aria-hidden />
            {tab.label}
            <span className="query-result-count mono">{tab.set.rows.length}</span>
          </button>
        ))}
        <button
          role="tab"
          aria-selected={shown === null}
          className={failed ? "query-result-tab has-problem" : "query-result-tab"}
          onClick={() => onActiveChange(MESSAGES)}
        >
          {failed && <TriangleAlert aria-hidden />}
          消息
        </button>
      </div>
      <div className="query-result-body" role="tabpanel">
        {shown ? (
          <ResultGrid key={shown.key} set={shown.set} label={`${title} 的${shown.label}`} />
        ) : (
          <MessagesView run={run} />
        )}
      </div>
    </section>
  );
}
