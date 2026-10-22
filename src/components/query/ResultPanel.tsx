import { FileDown, Table2, TriangleAlert } from "lucide-react";
import { useState } from "react";
import type { ResultSort } from "../../lib/queryTypes";
import { Button } from "../ui/Button";
import { MessagesView } from "./MessagesView";
import { ResultGrid } from "./ResultGrid";
import { resultTabs, type ResultTab, type RunState } from "./runModel";
import type { SqlDialect } from "./splitStatements";
import { shortcut } from "../../lib/platform";

export const MESSAGES = "messages";

interface ResultPanelProps {
  title: string;
  run: RunState;
  /** 选中的标签;null 或找不到时显示第一个。 */
  active: string | null;
  onActiveChange(key: string): void;
  /** 导出选中的结果集,`sort` 是它的表格上点的排序。 */
  onExport(tab: ResultTab, sort: ResultSort | null): void;
  editorId: string;
  connectionId: string;
  dialect: SqlDialect;
  /** 连接关着:结果先不让改。 */
  closed: boolean;
  /** 选中的结果里没提交的修改行数:有修改时不能换标签(换了表格就没了,修改也跟着丢)。 */
  pending: number;
  /** 提交修改后留给新结果的说明,按结果标签认。 */
  notice: { key: string; text: string } | null;
  onPendingChange(count: number): void;
  /** 结果里的修改提交了:重新执行这条语句。 */
  onCommitted(tab: ResultTab, message: string): Promise<void>;
}

const LOCKED = "先提交或撤销这个结果里的修改";

/** 编辑器下面的结果区:每个结果集一个标签,最后是「消息」。 */
export function ResultPanel(props: ResultPanelProps) {
  const { title, run, active, onActiveChange, onExport, pending } = props;
  // 显示中的结果表格上点的排序;换了结果、重新执行时表格重建,会重新报上来。
  const [sort, setSort] = useState<ResultSort | null>(null);
  const tabs = resultTabs(run.entries);
  if (run.entries.length === 0) {
    return (
      <section className="query-results is-empty" aria-label="执行结果">
        <p className="query-results-placeholder">
          {run.running
            ? "正在执行…"
            : `执行的结果会显示在这里。${shortcut("Enter")} 执行光标所在的语句，${shortcut("Enter", { shift: true })} 全部执行。`}
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
            title={pending > 0 && shown?.key !== tab.key ? LOCKED : tab.sql}
            disabled={pending > 0 && shown?.key !== tab.key}
            onClick={() => onActiveChange(tab.key)}
          >
            <Table2 aria-hidden />
            {tab.label}
            <span className="query-result-count mono">{tab.set.totalRows}</span>
          </button>
        ))}
        <button
          role="tab"
          aria-selected={shown === null}
          className={failed ? "query-result-tab has-problem" : "query-result-tab"}
          title={pending > 0 ? LOCKED : undefined}
          disabled={pending > 0}
          onClick={() => onActiveChange(MESSAGES)}
        >
          {failed && <TriangleAlert aria-hidden />}
          消息
        </button>
        {shown && (
          <Button
            className="query-result-export"
            variant="ghost"
            size="sm"
            icon={<FileDown />}
            disabled={run.running}
            title={
              shown.set.truncated
                ? "导出时会重新执行这条语句，读出全部结果"
                : pending > 0
                  ? "导出的是执行时读到的结果，不含还没提交的修改"
                  : "导出这个结果集"
            }
            onClick={() => onExport(shown, sort)}
          >
            导出
          </Button>
        )}
      </div>
      <div className="query-result-body" role="tabpanel">
        {shown ? (
          <ResultGrid
            key={`${shown.key}:${shown.set.id}`}
            editorId={props.editorId}
            connectionId={props.connectionId}
            dialect={props.dialect}
            sql={shown.sql}
            set={shown.set}
            label={`${title} 的${shown.label}`}
            frozen={run.running || props.closed}
            notice={props.notice?.key === shown.key ? props.notice.text : null}
            onPendingChange={props.onPendingChange}
            onCommitted={(message) => props.onCommitted(shown, message)}
            onSortChange={setSort}
          />
        ) : (
          <MessagesView run={run} />
        )}
      </div>
    </section>
  );
}
