import { useQueryClient } from "@tanstack/react-query";
import { RotateCcw, Save } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { catalogKeys, useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { designKeys, designQuery, useTableDesign } from "../../hooks/useTableDesign";
import { getBackend } from "../../lib/api";
import type { TableDesign } from "../../lib/designTypes";
import { errorMessage, isCommandError, type TableRef } from "../../lib/types";
import { tableDesignTab, useTabs, type TableDesignTab } from "../../stores/tabs";
import { DriverBadge } from "../connection/DriverBadge";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import { DdlPreviewDialog, type DdlPreviewState } from "./DdlPreviewDialog";
import { DesignEditors, DesignTabs, useSectionPrefix, type DesignSection } from "./DesignSections";
import * as model from "./designModel";
import { hasPrimaryModifier, shortcut } from "../../lib/platform";

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** 表结构设计器:改表(`tab.table` 有值)或新建表。⌘S 先出 SQL 预览,确认后执行。 */
export function TableDesignView({ tab, active }: { tab: TableDesignTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const isOpen = Boolean(connection?.isOpen);
  const { openWithFeedback } = useConnectionActions();
  const queryClient = useQueryClient();
  const ref: TableRef | null = useMemo(
    () => (tab.table === null ? null : { database: tab.database, schema: tab.schema, table: tab.table }),
    [tab.database, tab.schema, tab.table],
  );
  const query = useTableDesign(tab.connectionId, ref, isOpen);
  const original = ref ? (query.data ?? null) : null;

  // 改表:草稿从读出来的结构开始;新表:从一个自增主键开始。base 是草稿的比较基准。
  const [base, setBase] = useState<TableDesign | null>(null);
  const [draft, setDraft] = useState<TableDesign | null>(null);
  if (ref && original && base !== original) {
    // 结构重读了:没改过就跟着换;改过的草稿合并到新结构上(别人新加的东西保留,不当成用户删的)。
    if (!draft || !base || same(draft, base)) setDraft(original);
    else if (original.version !== base.version) setDraft(model.rebase(base, original, draft));
    setBase(original);
  }
  if (!ref && !base && connection) {
    const fresh = model.newTableDesign(connection.driver, tab.database, tab.schema);
    setBase(fresh);
    setDraft(fresh);
  }

  const prefix = useSectionPrefix();
  const [section, setSection] = useState<DesignSection>("columns");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [preview, setPreview] = useState<DdlPreviewState>({ phase: "closed" });
  const [message, setMessage] = useState<string | null>(null);

  const changes = draft && base ? (ref ? model.changeCount(base, draft) : same(draft, base) ? 0 : 1) : 0;
  const setUnsaved = useTabs((s) => s.setUnsaved);
  useEffect(() => {
    setUnsaved(tab.id, changes > 0);
  }, [setUnsaved, tab.id, changes]);
  useEffect(() => () => setUnsaved(tab.id, false), [setUnsaved, tab.id]);

  const edit = (next: TableDesign) => {
    setMessage(null);
    setDraft(next);
  };

  // ⌘S 可能在输入框里按下:下一轮再读最新的草稿。
  const latest = useRef({ draft, preview, base });
  useLayoutEffect(() => {
    latest.current = { draft, preview, base };
  });
  // 每打开或关闭一次预览加一:生成期间用户取消了,迟到的结果不能再把对话框弹回来。
  const previewToken = useRef(0);
  const closePreview = useCallback(() => {
    previewToken.current += 1;
    setPreview({ phase: "closed" });
  }, []);

  // 生成预览前先按库里的最新结构合并一次草稿:设计器开着的时候表可能被别人改过。
  const rebaseOnLatest = useCallback(
    async (table: TableRef) => {
      const fresh = await queryClient.fetchQuery({ ...designQuery(tab.connectionId, table), staleTime: 0 });
      const { base: was, draft: now } = latest.current;
      if (!was || !now || fresh.version === was.version) return now;
      const merged = same(now, was) ? fresh : model.rebase(was, fresh, now);
      setBase(fresh);
      setDraft(merged);
      return merged;
    },
    [queryClient, tab.connectionId, setBase, setDraft],
  );

  const openPreview = useCallback(async () => {
    const { draft: current, preview: shown } = latest.current;
    if (!current || shown.phase !== "closed") return;
    const token = ++previewToken.current;
    setPreview({ phase: "loading" });
    try {
      const design = ref ? await rebaseOnLatest(ref) : current;
      if (token !== previewToken.current || !design) return;
      const result = await (await getBackend()).previewTableDesign(tab.connectionId, ref, design);
      if (token === previewToken.current) {
        setPreview({ ...result, phase: "ready", design, applying: false, error: null, partial: false });
      }
    } catch (error) {
      if (token === previewToken.current) setPreview({ phase: "invalid", message: errorMessage(error) });
    }
  }, [ref, rebaseOnLatest, tab.connectionId]);

  const submit = useCallback(async () => {
    const shown = latest.current.preview;
    if (shown.phase !== "ready" || shown.applying) return;
    setPreview({ ...shown, applying: true, error: null });
    const tablesKey = catalogKeys.tables(tab.connectionId, tab.database, tab.schema);
    try {
      const backend = await getBackend();
      const outcome = await backend.applyTableDesign(tab.connectionId, ref, shown.design, shown.statements);
      await queryClient.invalidateQueries({ queryKey: tablesKey });
      if (!ref || outcome.table.table !== ref.table) {
        // 建好或改了名:换成新表名的设计器,它自己去读结构;新旧两个名字缓存的结构都作废。
        queryClient.removeQueries({ queryKey: designKeys.table(tab.connectionId, outcome.table) });
        if (ref) queryClient.removeQueries({ queryKey: designKeys.table(tab.connectionId, ref) });
        closePreview();
        const next = tableDesignTab(tab.connectionId, tab.database, tab.schema, outcome.table.table);
        useTabs.getState().replace(tab.id, next);
        return;
      }
      // 等重读完再收起对话框、换草稿,不闪回旧结构。
      const fresh = await queryClient.fetchQuery({
        queryKey: designKeys.table(tab.connectionId, ref),
        queryFn: () => backend.readTableDesign(tab.connectionId, ref),
        staleTime: 0,
      });
      setBase(fresh);
      setDraft(fresh);
      closePreview();
      setMessage(`已执行 ${outcome.statements} 条语句。`);
    } catch (error) {
      const partial = isCommandError(error) && error.code === "partially_applied";
      setPreview({ ...shown, applying: false, error: errorMessage(error), partial });
      if (partial) {
        // 一部分已经生效:重读结构,草稿留着,改好再存会按新结构重新比较。
        void queryClient.invalidateQueries({ queryKey: tablesKey });
        if (ref) void queryClient.invalidateQueries({ queryKey: designKeys.table(tab.connectionId, ref) });
      }
    }
  }, [closePreview, queryClient, ref, tab.connectionId, tab.database, tab.id, tab.schema, setBase, setDraft]);

  useEffect(() => {
    if (!active) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (hasPrimaryModifier(event) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "s") {
        event.preventDefault();
        window.setTimeout(() => void openPreview(), 0);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, openPreview]);

  const location = tab.schema ? `${tab.database}.${tab.schema}` : tab.database;
  const heading = tab.table ?? "新表";

  const body = !connection ? (
    <EmptyState title="这个连接已经不存在" />
  ) : !isOpen ? (
    <EmptyState
      title="连接已关闭"
      action={
        <Button variant="primary" onClick={() => void openWithFeedback(connection)}>
          重新打开
        </Button>
      }
    />
  ) : query.error && !original ? (
    <EmptyState
      title="读取表结构失败"
      detail={errorMessage(query.error)}
      tone="error"
      action={<Button onClick={() => void query.refetch()}>重试</Button>}
    />
  ) : !draft ? (
    <EmptyState title="正在读取表结构…" />
  ) : (
    <DesignEditors
      section={section}
      prefix={prefix}
      driver={connection.driver}
      connectionId={tab.connectionId}
      original={original}
      originalRef={ref}
      draft={draft}
      onChange={edit}
      selectedId={selectedId}
      onSelect={setSelectedId}
    />
  );

  return (
    <section className="designer" aria-label={`${heading} 的表结构`}>
      <header className="designer-header">
        <div className="designer-heading">
          {connection && <DriverBadge driver={connection.driver} />}
          <span className="data-crumb">{connection?.name ?? "已关闭的连接"}</span>
          <span className="data-crumb-sep">/</span>
          <span className="data-crumb">{location}</span>
          <span className="data-crumb-sep">/</span>
          <span className="designer-name" data-value={draft?.table.table || "输入表名"}>
            <input
              aria-label="表名"
              value={draft?.table.table ?? heading}
              placeholder="输入表名"
              disabled={!draft}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              autoFocus={!ref}
              onChange={(event) => draft && edit(model.renameTable(draft, event.target.value))}
            />
          </span>
          {ref && draft && draft.table.table !== ref.table && <span className="design-badge">改名</span>}
          {draft && (
            <span className="data-summary">
              {draft.columns.length} 个字段 ·{" "}
              {draft.primaryKey.length
                ? `主键 ${draft.primaryKey.map((id) => draft.columns.find((c) => c.id === id)?.name).join("、")}`
                : "无主键"}
            </span>
          )}
        </div>
        <div className="designer-bar">
          <DesignTabs section={section} onSelect={setSection} draft={draft} prefix={prefix} />
          <div className="data-actions">
            <Button
              size="sm"
              variant="ghost"
              icon={<RotateCcw />}
              disabled={changes === 0}
              onClick={() => {
                if (base) edit(base);
              }}
            >
              还原
            </Button>
            <Button
              size="sm"
              variant="primary"
              icon={<Save />}
              disabled={!draft || (ref !== null && changes === 0)}
              onClick={() => void openPreview()}
            >
              {ref ? "保存" : "创建"}
              {ref && changes > 0 && <span className="commit-count">{changes}</span>}
            </Button>
          </div>
        </div>
      </header>

      <div className="designer-body">{body}</div>

      <footer className="data-footer designer-footer">
        {changes > 0 ? (
          <span className="data-footer-pending">
            {ref ? `${changes} 处修改未保存` : "新表还没有创建"} · <kbd>{shortcut("S")}</kbd> 预览并保存
          </span>
        ) : (
          message && <span className="data-footer-message">{message}</span>
        )}
      </footer>

      <DdlPreviewDialog state={preview} onCancel={closePreview} onSubmit={() => void submit()} />
    </section>
  );
}
