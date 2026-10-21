// SQL 助手的对话,按查询标签页分开存:收起面板再打开,对话还在;关掉标签页就丢掉。
import { create } from "zustand";
import type { ChatRole } from "../lib/llmTypes";

export type TurnStatus = "streaming" | "done" | "error" | "cancelled";

export interface Turn {
  /** 助手的回答用请求 id,用户的话随便起。 */
  id: string;
  role: ChatRole;
  content: string;
  status: TurnStatus;
  error?: string;
}

interface AssistantStore {
  conversations: Readonly<Record<string, readonly Turn[]>>;
  drafts: Readonly<Record<string, string>>;
  /** 用户问了一句:记下这句和一条正在生成的回答。 */
  ask(tabId: string, question: Turn, answer: Turn): void;
  append(tabId: string, turnId: string, text: string): void;
  /** 回答结束。`content` 给了就以它为准(完整的回答)。 */
  settle(
    tabId: string,
    turnId: string,
    status: Exclude<TurnStatus, "streaming">,
    extra?: { error?: string; content?: string },
  ): void;
  setDraft(tabId: string, text: string): void;
  clear(tabId: string): void;
  /** 标签页关了:对话和草稿一起丢掉。 */
  forget(tabId: string): void;
}

const updateTurn = (turns: readonly Turn[], id: string, update: (turn: Turn) => Turn) =>
  turns.map((turn) => (turn.id === id ? update(turn) : turn));

/** 标签页已经关了(对话被丢掉)时,迟到的片段、结束通知都不再写回去。 */
const changeTurn =
  (tabId: string, turnId: string, update: (turn: Turn) => Turn) =>
  (s: AssistantStore): Partial<AssistantStore> => {
    const turns = s.conversations[tabId];
    if (!turns) return {};
    return { conversations: { ...s.conversations, [tabId]: updateTurn(turns, turnId, update) } };
  };

export const useAssistant = create<AssistantStore>((set) => ({
  conversations: {},
  drafts: {},
  ask: (tabId, question, answer) =>
    set((s) => ({
      conversations: { ...s.conversations, [tabId]: [...(s.conversations[tabId] ?? []), question, answer] },
    })),
  append: (tabId, turnId, text) => set(changeTurn(tabId, turnId, (t) => ({ ...t, content: t.content + text }))),
  settle: (tabId, turnId, status, extra = {}) =>
    set(
      changeTurn(tabId, turnId, (t) => ({
        ...t,
        status,
        error: extra.error,
        content: extra.content ?? t.content,
      })),
    ),
  setDraft: (tabId, text) => set((s) => ({ drafts: { ...s.drafts, [tabId]: text } })),
  clear: (tabId) => set((s) => ({ conversations: { ...s.conversations, [tabId]: [] } })),
  forget: (tabId) =>
    set((s) => {
      const { [tabId]: _conversation, ...conversations } = s.conversations;
      const { [tabId]: _draft, ...drafts } = s.drafts;
      return { conversations, drafts };
    }),
}));

/** 这个标签页正在生成的回答的请求 id(回答的 id 就是请求 id);没有时为 null。 */
export function streamingRequest(tabId: string): string | null {
  const turns = useAssistant.getState().conversations[tabId] ?? [];
  return turns.find((t) => t.role === "assistant" && t.status === "streaming")?.id ?? null;
}
