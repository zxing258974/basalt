import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { savedQueryKeys } from "../../hooks/useSavedQueries";
import { getBackend } from "../../lib/api";
import type { QueryScope, SavedQueryRef } from "../../lib/queryTypes";
import { errorMessage } from "../../lib/types";
import { useAssistant } from "../../stores/assistant";
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
import { resultTabs, statementsToRun, type RunMode } from "./runModel";
import { SaveQueryDialog } from "./SaveQueryDialog";
import { editorDialect, SqlEditor, type SqlEditorHandle } from "./SqlEditor";
import { useCompletionProvider } from "./useCompletionProvider";
import { useEditorHeight } from "./useEditorHeight";
import { useQueryRunner } from "./useQueryRunner";
import { hasPrimaryModifier } from "../../lib/platform";

/** 和保存时的内容比;没保存过的,写了东西就算有改动。 */
const isDirty = (text: string, baseline: string, saved: boolean) => (saved ? text !== baseline : text.trim() !== "");

export function QueryView({ tab, active }: { tab: QueryTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const driver = connection?.driver ?? null;
  const isOpen = Boolean(connection?.isOpen);
  const queryClient = useQueryClient();
  const { openWithFeedback } = useConnectionActions();
  const { updateQuery, setUnsaved } = useTabs.getState();
  const editor = useRef<SqlEditorHandle>(null);
  const baseline = useRef(tab.initialSql);
  const [dirty, setDirty] = useState(false);
  const [activeResult, setActiveResult] = useState<string | null>(null);
  /** 执行过程中用户自己点了结果标签:执行完就不再替他切换。 */
  const pickedDuringRun = useRef(false);
  /** 右侧开着哪个面板:执行历史和 SQL 助手二选一,窄窗口里不至于挤掉编辑器。 */
  const [side, setSide] = useState<"history" | "assistant" | null>(null);
  const toggleSide = (panel: "history" | "assistant") => setSide((open) => (open === panel ? null : panel));
  const [saveAs, setSaveAs] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const { run, execute, cancel } = useQueryRunner(tab);
  const provider = useCompletionProvider(tab.connectionId, driver, tab.scope);
  const { height, handlers } = useEditorHeight();

  const markDirty = useCallback(
    (next: boolean) => {
      setDirty(next);
      setUnsaved(tab.id, next);
    },
    [setUnsaved, tab.id],
  );
  const onChange = useCallback(
    (text: string) => markDirty(isDirty(text, baseline.current, tab.saved !== null)),
    [markDirty, tab.saved],
  );

  // 标签页关了:放掉它的专用连接,也不再算「没保存」,助手的对话也丢掉。
  useEffect(
    () => () => {
      useTabs.getState().setUnsaved(tab.id, false);
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
      const statements = statementsToRun(handle.getText(), handle.getSelection(), editorDialect(driver), mode);
      if (statements.length === 0) {
        setHint(mode === "all" ? "编辑器里没有可执行的语句。" : "光标所在的位置没有语句。");
        return;
      }
      setHint(null);
      setActiveResult(null);
      pickedDuringRun.current = false;
      void execute(statements).then((done) => {
        if (!done || pickedDuringRun.current) return;
        const failed = done.entries.some((e) => e.failure !== null || e.outcome?.status !== "ok");
        const first = resultTabs(done.entries)[0];
        setActiveResult(failed || !first ? MESSAGES : first.key);
      });
    },
    [driver, execute, isOpen, run.running],
  );

  const writeQuery = useCallback(
    async (ref: SavedQueryRef, overwrite: boolean) => {
      const text = editor.current?.getText() ?? "";
      await (await getBackend()).saveQuery(ref, text, overwrite);
      baseline.current = text;
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
        className={
          side === "assistant" ? "query-body has-history has-assistant" : side ? "query-body has-history" : "query-body"
        }
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
            onExport={(result) =>
              useTransfer.getState().openExport({
                connectionId: tab.connectionId,
                driver: driver ?? "mysql",
                title: `${tab.title} 的${result.label}`,
                baseName: guessTableName(result.sql),
                scope: { kind: "query", editorId: tab.id, scope: result.scope, sql: result.sql, set: result.set },
              })
            }
          />
        </div>
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
