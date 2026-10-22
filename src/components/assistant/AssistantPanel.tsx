import { useQueryClient } from "@tanstack/react-query";
import { CircleStop, MessageSquarePlus, Settings2, Sparkles, Wand2, X } from "lucide-react";
import { AssistantMark } from "./AssistantMark";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useSchemaOutline, useServerInfo } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import type { QueryScope } from "../../lib/queryTypes";
import { errorMessage, type ConnectionView } from "../../lib/types";
import { useAssistant, type Turn } from "../../stores/assistant";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { AssistantMessages } from "./AssistantMessages";
import { optimizeMessage, systemPrompt } from "./assistantModel";
import { LLM_SETTINGS_KEY, LlmSettingsDialog, useLlmSettings } from "./LlmSettingsDialog";
import { useAssistantChat } from "./useAssistantChat";
import { hasPrimaryModifier, shortcut } from "../../lib/platform";

/** 给读屏软件的一句:只在开始、结束时变,不逐字念。 */
function announcement(turn: Turn | undefined): string {
  if (turn?.role !== "assistant") return "";
  switch (turn.status) {
    case "streaming":
      return "助手正在生成回答…";
    case "done":
      return "助手的回答完成了。";
    case "cancelled":
      return "助手的回答停下了。";
    case "error":
      return `助手出错了：${turn.error ?? ""}`;
  }
}

interface AssistantPanelProps {
  tabId: string;
  connection: ConnectionView | undefined;
  scope: QueryScope;
  /** 要优化的 SQL:编辑器里选中的部分,没选中就是全部。 */
  readSql(): string;
  /** 插到编辑器的光标处。只插入,不执行。 */
  onInsert(sql: string): void;
  onClose(): void;
}

/** 查询页右侧的 SQL 助手:按问题生成 SQL、优化当前的 SQL。它只写 SQL,从不执行。 */
export function AssistantPanel({ tabId, connection, scope, readSql, onInsert, onClose }: AssistantPanelProps) {
  const queryClient = useQueryClient();
  const settings = useLlmSettings();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const { turns, streaming, send, stop } = useAssistantChat(tabId);
  const draft = useAssistant((s) => s.drafts[tabId] ?? "");
  const setDraft = (text: string) => useAssistant.getState().setDraft(tabId, text);
  const list = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);

  const driver = connection?.driver ?? "mysql";
  const isOpen = Boolean(connection?.isOpen);
  const schema = driver === "postgres" ? scope.schema : null;
  const server = useServerInfo(connection?.id ?? null);
  const outline = useSchemaOutline(connection?.id ?? "", scope.database, schema, isOpen);
  const models = settings.data?.models ?? [];
  const model = models.find((m) => m.id === settings.data?.active) ?? models[0];

  // 新的一段到了就滚到底,跟着回答往下走。
  const last = turns.at(-1);
  useEffect(() => {
    const element = list.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [turns.length, last?.content.length]);

  const ask = (kind: "generate" | "optimize") => {
    if (!model) {
      setSettingsOpen(true);
      return;
    }
    let question: string;
    if (kind === "optimize") {
      const sql = readSql();
      if (!sql.trim()) {
        setHint("编辑器里还没有 SQL：先写一条，或选中要优化的那部分。");
        return;
      }
      question = optimizeMessage(sql, draft);
    } else {
      if (!draft.trim()) {
        setHint("先写下你想查什么。");
        return;
      }
      question = draft.trim();
    }
    setHint(null);
    setDraft("");
    // 点按钮发出去之后焦点回到输入框:按钮在生成期间会变成不可用,焦点不能丢到页面上。
    input.current?.focus();
    const system = systemPrompt(
      {
        driver,
        version: server.data?.version ?? null,
        database: scope.database,
        schema,
        outline: outline.data ?? [],
      },
      question,
    );
    void send(model, system, question);
  };

  const pickModel = async (id: string) => {
    try {
      queryClient.setQueryData(LLM_SETTINGS_KEY, await (await getBackend()).setActiveLlmModel(id));
    } catch (error) {
      useUi.getState().notify(`切换模型失败：${errorMessage(error)}`);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // ⌘↩ 在这里是「发送」:preventDefault 让查询页不再把它当成「执行 SQL」。
    if (hasPrimaryModifier(event) && event.key === "Enter") {
      event.preventDefault();
      if (!streaming) ask("generate");
    }
  };

  const context = !connection
    ? "连接已删除，助手拿不到表结构。"
    : !isOpen
      ? "连接没打开，助手拿不到表结构。"
      : !scope.database
        ? "还没选库：在上面选一个库，助手才知道有哪些表。"
        : outline.isPending
          ? "正在读表结构…"
          : outline.error
            ? `读不到表结构：${errorMessage(outline.error)}`
            : `会附带「${schema ?? scope.database}」里 ${outline.data?.length ?? 0} 张表的结构（不含数据）。`;

  return (
    <aside className="assistant" aria-label="SQL 助手">
      <header className="assistant-header">
        <h2 className="assistant-title">
          <AssistantMark />
          SQL 助手
        </h2>
        {models.length > 0 && (
          <select
            className="assistant-model"
            aria-label="模型"
            value={model?.id ?? ""}
            disabled={streaming}
            onChange={(event) => void pickModel(event.target.value)}
          >
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        )}
        <span className="assistant-header-actions">
          <Button
            variant="ghost"
            size="sm"
            icon={<MessageSquarePlus />}
            aria-label="新对话"
            title="新对话"
            disabled={turns.length === 0 || streaming}
            onClick={() => useAssistant.getState().clear(tabId)}
          />
          <Button
            variant="ghost"
            size="sm"
            icon={<Settings2 />}
            aria-label="模型设置"
            title="模型设置"
            onClick={() => setSettingsOpen(true)}
          />
          <Button variant="ghost" size="sm" icon={<X />} aria-label="关闭 SQL 助手" onClick={onClose} />
        </span>
      </header>

      <div className="assistant-body" ref={list}>
        {settings.isSuccess && models.length === 0 ? (
          <div className="assistant-empty">
            <p>还没有配置模型。填上接口地址、模型名称和 API Key 就能用，支持 OpenAI 兼容接口和 Claude 接口。</p>
            <Button variant="primary" size="sm" icon={<Settings2 />} onClick={() => setSettingsOpen(true)}>
              配置模型…
            </Button>
          </div>
        ) : turns.length === 0 ? (
          <div className="assistant-empty">
            <p>用一句话描述你要查的数据，助手会按这个库的表结构写出 SQL；也可以让它优化编辑器里的 SQL。</p>
          </div>
        ) : (
          <AssistantMessages turns={turns} onInsert={onInsert} />
        )}
      </div>

      <div className="assistant-composer">
        <p className="assistant-context">{context}</p>
        <textarea
          ref={input}
          className="assistant-input"
          aria-label="问助手"
          placeholder={`如：查最近 30 天每天的订单数和金额（${shortcut("Enter")} 发送）`}
          value={draft}
          rows={3}
          onChange={(event) => {
            setDraft(event.target.value);
            setHint(null);
          }}
          onKeyDown={onKeyDown}
        />
        {hint && (
          <p className="assistant-hint" role="status">
            {hint}
          </p>
        )}
        <div className="assistant-actions">
          <Button
            variant="primary"
            size="sm"
            icon={<Sparkles />}
            title={shortcut("Enter")}
            disabled={streaming}
            onClick={() => ask("generate")}
          >
            生成 SQL
          </Button>
          <Button
            size="sm"
            icon={<Wand2 />}
            title="优化编辑器里选中的 SQL；没选中就是全部。输入框里写的会作为补充说明"
            disabled={streaming}
            onClick={() => ask("optimize")}
          >
            优化当前 SQL
          </Button>
          {streaming && (
            <Button
              variant="danger"
              size="sm"
              icon={<CircleStop />}
              onClick={() => {
                void stop();
                input.current?.focus();
              }}
            >
              停止
            </Button>
          )}
        </div>
        <p className="visually-hidden" aria-live="polite">
          {announcement(last)}
        </p>
        <p className="assistant-note">助手只写 SQL，不会执行。复制或插入后，由你自己检查、执行。</p>
      </div>
      {settingsOpen && <LlmSettingsDialog onClose={() => setSettingsOpen(false)} />}
    </aside>
  );
}
