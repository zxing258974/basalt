// 助手发问、收流式回答、停止。回答一段段写进 assistant store,按查询标签页分开。
import { useCallback } from "react";
import { getBackend } from "../../lib/api";
import type { ChatMessage, LlmModel } from "../../lib/llmTypes";
import { errorMessage, type CommandError } from "../../lib/types";
import { streamingRequest, useAssistant, type Turn } from "../../stores/assistant";
import { trimHistory } from "./assistantModel";

const NO_TURNS: readonly Turn[] = [];

/** 只把答完了的一问一答当上下文:出错、停掉的那一问连同回答一起不算,免得连着两句用户的话。 */
export function completedPairs(turns: readonly Turn[]): ChatMessage[] {
  const messages: ChatMessage[] = [];
  for (let i = 0; i + 1 < turns.length; i += 2) {
    const [question, answer] = [turns[i]!, turns[i + 1]!];
    const answered = answer.status === "done" && answer.content.trim() !== "";
    if (question.role === "user" && answer.role === "assistant" && answered) {
      messages.push({ role: "user", content: question.content }, { role: "assistant", content: answer.content });
    }
  }
  return messages;
}

export function useAssistantChat(tabId: string) {
  const turns = useAssistant((s) => s.conversations[tabId] ?? NO_TURNS);
  const streaming = turns.some((t) => t.status === "streaming");

  const send = useCallback(
    async (model: LlmModel, system: string, question: string) => {
      const store = useAssistant.getState();
      const history = trimHistory(completedPairs(store.conversations[tabId] ?? NO_TURNS));
      const requestId = crypto.randomUUID();
      store.ask(
        tabId,
        { id: crypto.randomUUID(), role: "user", content: question, status: "done" },
        { id: requestId, role: "assistant", content: "", status: "streaming" },
      );
      const backend = await getBackend();
      // 先订阅再发请求:第一段不会漏掉。
      const unlisten = await backend.onLlmChat((delta) => {
        if (delta.requestId === requestId) useAssistant.getState().append(tabId, requestId, delta.text);
      });
      try {
        const outcome = await backend.llmChat(requestId, {
          modelId: model.id,
          system,
          messages: [...history, { role: "user", content: question }],
        });
        useAssistant.getState().settle(tabId, requestId, "done", { content: outcome.text });
      } catch (error) {
        const cancelled = (error as CommandError).code === "cancelled";
        useAssistant
          .getState()
          .settle(tabId, requestId, cancelled ? "cancelled" : "error", cancelled ? {} : { error: errorMessage(error) });
      } finally {
        unlisten();
      }
    },
    [tabId],
  );

  // 从 store 里找正在生成的那条:面板关掉再打开(组件换了一个)也停得了。
  const stop = useCallback(() => cancelStreaming(tabId), [tabId]);

  return { turns, streaming, send, stop };
}

/** 停掉这个标签页正在生成的回答(关标签页时也用)。 */
export async function cancelStreaming(tabId: string): Promise<void> {
  const requestId = streamingRequest(tabId);
  if (!requestId) return;
  try {
    await (await getBackend()).cancelLlmChat(requestId);
  } catch (error) {
    console.warn("停止助手回答失败", error);
  }
}
