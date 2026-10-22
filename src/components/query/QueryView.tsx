import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { savedQueryKeys } from "../../hooks/useSavedQueries";
import { getBackend } from "../../lib/api";
import type { QueryScope, SavedQueryRef } from "../../lib/queryTypes";
import { errorMessage } from "../../lib/types";
import { useAssistant } from "../../stores/assistant";
import { useWorkspace } from "../../stores/workspace";
import { useTabs, type QueryTab } from "../../stores/tabs";
import { useTransfer } from "../../stores/transfer";
import { guessTableName } from "../transfer/exportModel";
import { useUi } from "../../stores/ui";
import { AssistantPanel } from "../assistant/AssistantPanel";
import { cancelStreaming } from "../assistant/useAssistantChat";
import { Button } from "../ui/Button";
import { HistoryPanel } from "./HistoryPanel";
import { QueryHeader } from "./QueryHeader";
import { MESSAGES, ResultPanel } from "./ResultPanel";
import { resultTabs, statementsToRun, type ResultTab, type RunMode, type RunState } from "./runModel";
import { SaveQueryDialog } from "./SaveQueryDialog";
import { formatEdit } from "./formatSql";
import { editorDialect, SqlEditor, type SqlEditorHandle } from "./SqlEditor";
import type { SqlStatement } from "./splitStatements";
import { useCompletionProvider } from "./useCompletionProvider";
import { useEditorHeight } from "./useEditorHeight";
import { usePanelWidth, type PanelWidthOptions } from "../../hooks/usePanelWidth";
import { useQueryRunner } from "./useQueryRunner";
import { hasPrimaryModifier } from "../../lib/platform";

/** 和保存时的内容比;没保存过的,写了东西就算有改动。 */
const isDirty = (text: string, baseline: string, saved: boolean) => (saved ? text !== baseline : text.trim() !== "");

/** 右侧栏两种面板各记各的宽度:助手要放代码,通常比历史宽。 */
const HISTORY_PANEL: PanelWidthOptions = {
  storageKey: "basalt.historyWidth",
  defaultWidth: 300,
  min: 240,
  max: 640,
  edge: "left",
  label: "调整执行历史宽度",
};
const ASSISTANT_PANEL: PanelWidthOptions = {
  storageKey: "basalt.assistantWidth",
  defaultWidth: 340,
  min: 280,
  max: 720,
  edge: "left",
  label: "调整 SQL 助手宽度",
};

/**
 * 第一条出错的语句,数据库说了错在哪就在编辑器里标出来。执行期间改过编辑器就不标:位置已经对不上了。
 */
function markFirstError(handle: SqlEditorHandle, done: RunState, statements: SqlStatement[], textAtRun: string) {
  if (handle.getText() !== textAtRun) return;
  const index = done.entries.findIndex((entry) => entry.outcome?.status === "error");
  const outcome = done.entries[index]?.outcome;
  const statement = statements[index];
  if (!outcome || outcome.errorPosition === null || !statement) return;
  handle.markError({ at: statement.from + outcome.errorPosition, end: statement.to, message: outcome.message ?? "" });
}

export function QueryView({ tab, active }: { tab: QueryTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const driver = connection?.driver ?? null;
  const isOpen = Boolean(connection?.isOpen);
  const queryClient = useQueryClient();
  const { openWithFeedback } = useConnectionActions();
  const { updateQuery, setUnsaved } = useTabs.getState();
  const editor = useRef<SqlEditorHandle>(null);
  // 上次保存(或打开)时的内容;从上次的记录恢复时另有记录,改过没保存的照样标出来。
  const baseline = useRef(tab.baselineSql ?? tab.initialSql);
  const [dirty, setDirty] = useState(false);
  const [activeResult, setActiveResult] = useState<string | null>(null);
  /** 执行过程中用户自己点了结果标签:执行完就不再替他切换。 */
  const pickedDuringRun = useRef(false);
  /** 右侧开着哪个面板:执行历史和 SQL 助手二选一,窄窗口里不至于挤掉编辑器。 */
  const [side, setSide] = useState<"history" | "assistant" | null>(null);
  const toggleSide = (panel: "history" | "assistant") => setSide((open) => (open === panel ? null : panel));
  const [saveAs, setSaveAs] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const { run, execute, rerun, cancel } = useQueryRunner(tab);
  /** 选中的结果里没提交的修改行数。 */
  const [resultPending, setResultPending] = useState(0);
  /** 结果里的修改提交后,换上的新结果要显示的说明。 */
  const [resultNotice, setResultNotice] = useState<{ key: string; text: string } | null>(null);
  const provider = useCompletionProvider(tab.connectionId, driver, tab.scope);
  const { height, handlers } = useEditorHeight();
  const historyWidth = usePanelWidth(HISTORY_PANEL);
  const assistantWidth = usePanelWidth(ASSISTANT_PANEL);
  const sidePanel = side === "assistant" ? assistantWidth : side === "history" ? historyWidth : null;

  const markDirty = useCallback(
    (next: boolean) => {
      setDirty(next);
      setUnsaved(tab.id, next);
    },
    [setUnsaved, tab.id],
  );
  const onChange = useCallback(
    (text: string) => {
      useWorkspace.getState().setDraft(tab.id, { sql: text });
      markDirty(isDirty(text, baseline.current, tab.saved !== null));
    },
    [markDirty, tab.id, tab.saved],
  );

  // 编辑器里的内容交给标签页记录(退出再打开还在);恢复回来的、改过没保存的,一打开就标出来。
  useEffect(() => {
    useWorkspace.getState().setDraft(tab.id, { sql: tab.initialSql, baseline: baseline.current });
    if (isDirty(tab.initialSql, baseline.current, tab.saved !== null)) markDirty(true);
    // 只在打开时做一次:之后的内容由 onChange 跟着记。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.id]);

  // 结果里没提交的修改算进标签页:关标签页、关连接、退出前都要确认。
  useEffect(() => {
    useTabs.getState().setPending(tab.id, resultPending);
  }, [resultPending, tab.id]);
  useEffect(() => () => useTabs.getState().setPending(tab.id, 0), [tab.id]);

  // 标签页关了:放掉它的专用连接,也不再算「没保存」,助手的对话也丢掉。
  useEffect(
    () => () => {
      useTabs.getState().setUnsaved(tab.id, false);
      useWorkspace.getState().forgetDraft(tab.id);
      // 正在生成的回答也停掉:标签页没了,回答没地方放,再生成下去只是白白消耗额度。
      void cancelStreaming(tab.id).finally(() => useAssistant.getState().forget(tab.id));
      void getBackend()
        .then((backend) => backend.closeEditor(tab.id))
        .catch((error: unknown) => console.warn("断开查询标签页的连接失败", error));
    },
    [tab.id],
  );

  const runStatements = useCallback(
    (mode: RunMode) => {
      const handle = editor.current;
      if (!handle || run.running || !isOpen) return;
      if (resultPending > 0) {
        setHint("结果里还有没提交的修改：先提交或撤销，再执行。");
        return;
      }
      const statements = statementsToRun(handle.getText(), handle.getSelection(), editorDialect(driver), mode);
      if (statements.length === 0) {
        setHint(mode === "all" ? "编辑器里没有可执行的语句。" : "光标所在的位置没有语句。");
        return;
      }
      setHint(null);
      setResultNotice(null);
      setActiveResult(null);
      pickedDuringRun.current = false;
      handle.markError(null);
      const textAtRun = handle.getText();
      void execute(statements).then((done) => {
        if (done && editor.current) markFirstError(editor.current, done, statements, textAtRun);
        if (!done || pickedDuringRun.current) return;
        const failed = done.entries.some((e) => e.failure !== null || e.outcome?.status !== "ok");
        const first = resultTabs(done.entries)[0];
        setActiveResult(failed || !first ? MESSAGES : first.key);
      });
    },
    [driver, execute, isOpen, resultPending, run.running],
  );

  /** 美化选中的部分或整个编辑器。等格式化程序加载的时候改了内容,就不换了(会盖掉刚打的字)。 */
  const format = useCallback(async () => {
    const handle = editor.current;
    if (!handle) return;
    const text = handle.getText();
    try {
      const result = await formatEdit(text, handle.getSelection(), driver);
      if (handle.getText() !== text) return;
      if (result.kind === "edit") {
        handle.replace(result);
        setHint(null);
      } else if (result.kind === "unchanged") {
        setHint("已经是美化过的样子了。");
      } else {
        setHint(result.selection ? "选中的部分没有 SQL。" : "编辑器里没有 SQL。");
      }
    } catch (error) {
      setHint(`没法美化：${errorMessage(error)}`);
    }
  }, [driver]);

  const onCommitted = useCallback(
    async (result: ResultTab, message: string) => {
      setResultNotice({ key: result.key, text: message });
      await rerun(result.entry);
    },
    [rerun],
  );

  const writeQuery = useCallback(
    async (ref: SavedQueryRef, overwrite: boolean) => {
      const text = editor.current?.getText() ?? "";
      await (await getBackend()).saveQuery(ref, text, overwrite);
      baseline.current = text;
      useWorkspace.getState().setDraft(tab.id, { sql: text, baseline: text });
      updateQuery(tab.id, { saved: ref, title: ref.name });
      markDirty(false);
      setSaveAs(false);
      await queryClient.invalidateQueries({
        queryKey: savedQueryKeys.list(ref.connectionId, ref.database, ref.schema),
      });
    },
    [markDirty, queryClient, tab.id, updateQuery],
  );

  const save = useCallback(
    async (forceDialog: boolean) => {
      if (tab.saved && !forceDialog) {
        try {
          await writeQuery(tab.saved, true);
          setHint(`已保存到「${tab.saved.name}」。`);
        } catch (error) {
          useUi.getState().notify(`保存失败：${errorMessage(error)}`);
        }
        return;
      }
      if (!tab.scope.database) {
        setHint("先在上面选一个库：查询会保存在这个库下面。");
        return;
      }
      if (driver === "postgres" && !tab.scope.schema) {
        setHint("先在上面选一个 schema：PostgreSQL 的查询保存在 schema 下面。");
        return;
      }
      setSaveAs(true);
    },
    [driver, tab.saved, tab.scope.database, tab.scope.schema, writeQuery],
  );

  // 标签页可见时:⌘S 保存、⇧⌘S 另存为;编辑器没获得焦点时 ⌘↩ 也能执行。
  useEffect(() => {
    if (!active) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!hasPrimaryModifier(event) || event.altKey || event.defaultPrevented) return;
      const key = event.key.toLowerCase();
      if (key === "s") {
        event.preventDefault();
        void save(event.shiftKey);
      } else if (key === "enter" && !(event.target instanceof HTMLInputElement)) {
        event.preventDefault();
        runStatements(event.shiftKey ? "all" : "current");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, runStatements, save]);

  const changeScope = (scope: QueryScope) => updateQuery(tab.id, { scope });
  // 另存为总是存到标签页当前所在的库(PG 是 schema)下。
  const location = {
    connectionId: tab.connectionId,
    database: tab.scope.database ?? "",
    schema: driver === "postgres" ? tab.scope.schema : null,
  };

  return (
    <section className="query-view" aria-label={`${tab.title}（SQL 查询）`}>
      <QueryHeader
        connection={connection}
        scope={tab.scope}
        title={tab.title}
        dirty={dirty}
        running={run.running}
        cancelling={run.cancelRequested}
        historyOpen={side === "history"}
        assistantOpen={side === "assistant"}
        onScopeChange={changeScope}
        onRun={() => runStatements("current")}
        onRunAll={() => runStatements("all")}
        onCancel={() => void cancel()}
        onSave={() => void save(false)}
        onFormat={() => void format()}
        onToggleHistory={() => toggleSide("history")}
        onToggleAssistant={() => toggleSide("assistant")}
      />
      {!connection && (
        <p className="data-notice">这个连接已经删除了。查询内容还在，可以复制出去；没法再执行或保存到原处。</p>
      )}
      {connection && !isOpen && (
        <p className="data-notice">
          连接已关闭，打开后才能执行。
          <Button size="sm" onClick={() => void openWithFeedback(connection)}>
            重新打开
          </Button>
        </p>
      )}
      {hint && (
        <p className="query-hint" role="status">
          {hint}
        </p>
      )}
      <div
        className={sidePanel ? "query-body has-side" : "query-body"}
        style={sidePanel ? ({ "--query-side-width": `${sidePanel.width}px` } as React.CSSProperties) : undefined}
      >
        <div className="query-main" style={{ "--editor-height": `${height}px` } as React.CSSProperties}>
          <SqlEditor
            ref={editor}
            driver={driver}
            initialText={tab.initialSql}
            label={`${tab.title} 的 SQL`}
            completion={provider}
            onChange={onChange}
            onRun={() => runStatements("current")}
            onRunAll={() => runStatements("all")}
            onCancel={() => void cancel()}
            onFormat={() => void format()}
          />
          <div
            className="query-splitter"
            role="separator"
            aria-orientation="horizontal"
            aria-label="调整编辑器高度"
            aria-valuenow={height}
            {...handlers}
          />
          <ResultPanel
            title={tab.title}
            run={run}
            active={activeResult}
            onActiveChange={(key) => {
              pickedDuringRun.current = true;
              setActiveResult(key);
            }}
            onExport={(result, sort) =>
              useTransfer.getState().openExport({
                connectionId: tab.connectionId,
                driver: driver ?? "mysql",
                title: `${tab.title} 的${result.label}`,
                baseName: guessTableName(result.sql),
                scope: { kind: "query", editorId: tab.id, scope: result.scope, sql: result.sql, set: result.set, sort },
              })
            }
            editorId={tab.id}
            connectionId={tab.connectionId}
            dialect={editorDialect(driver)}
            closed={!isOpen}
            pending={resultPending}
            notice={resultNotice}
            onPendingChange={setResultPending}
            onCommitted={onCommitted}
          />
        </div>
        {sidePanel && <div className="panel-resizer" {...sidePanel.separator} />}
        {side === "history" && (
          <HistoryPanel
            connectionId={tab.connectionId}
            onPick={(sql) => editor.current?.insert(sql)}
            onClose={() => setSide(null)}
          />
        )}
        {side === "assistant" && (
          <AssistantPanel
            tabId={tab.id}
            connection={connection}
            scope={tab.scope}
            readSql={() => {
              const handle = editor.current;
              if (!handle) return "";
              const text = handle.getText();
              const { from, to } = handle.getSelection();
              return from === to ? text : text.slice(from, to);
            }}
            onInsert={(sql) => {
              editor.current?.insert(sql);
              editor.current?.focus();
            }}
            onClose={() => setSide(null)}
          />
        )}
      </div>
      {saveAs && (
        <SaveQueryDialog
          location={location}
          connectionName={connection?.name ?? tab.connectionId}
          initialName={tab.saved?.name ?? ""}
          onSave={writeQuery}
          onClose={() => setSaveAs(false)}
        />
      )}
    </section>
  );
}
