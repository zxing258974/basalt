// mock 后端的 SQL 助手:模型配置存在内存里;对话按问题编一段带 SQL 的回答,几个字一段推出来,可以取消。
import type { Backend } from "../api";
import type { ChatDelta, ChatRequest, LlmModel, LlmModelDraft, LlmSettings } from "../llmTypes";
import type { CommandError, FieldError } from "../types";

type LlmApi = Pick<
  Backend,
  | "listLlmModels"
  | "saveLlmModel"
  | "deleteLlmModel"
  | "setActiveLlmModel"
  | "testLlmModel"
  | "llmChat"
  | "cancelLlmChat"
  | "onLlmChat"
>;

export const SAMPLE_MODEL: LlmModel = {
  id: "mock-deepseek",
  name: "DeepSeek",
  protocol: "openai",
  baseUrl: "https://api.deepseek.com/v1",
  model: "deepseek-chat",
  maxTokens: null,
  temperature: 0.2,
  hasApiKey: true,
};

const GENERATED = `按下单时间倒序列出最近 30 天的订单，带上客户名：

\`\`\`sql
SELECT o.id, c.name AS customer, o.total, o.created_at
FROM orders o
JOIN customers c ON c.id = o.customer_id
WHERE o.created_at >= NOW() - INTERVAL 30 DAY
ORDER BY o.created_at DESC
LIMIT 100;
\`\`\`

说明：用 customer_id 关联 customers 取名字；默认只取 100 行，要全部数据可以去掉 LIMIT。`;

const OPTIMIZED = `原语句对 created_at 套了函数，用不上索引，会全表扫描。改成范围条件：

\`\`\`sql
SELECT id, customer_id, total
FROM orders
WHERE created_at >= '2026-10-01' AND created_at < '2026-11-01';
\`\`\`

如果经常按时间查，可以建一个索引：

\`\`\`sql
CREATE INDEX idx_orders_created_at ON orders (created_at);
\`\`\``;

const CHUNK = 6;

function fail(code: string, message: string, fields: FieldError[] = []): never {
  const error: CommandError = { code, message, fields };
  throw error;
}

function check(draft: LlmModelDraft): void {
  const fields: FieldError[] = [];
  if (!draft.name.trim()) fields.push({ field: "name", message: "起个名字，方便在助手里选。" });
  if (!/^https?:\/\/[^/\s]+/.test(draft.baseUrl.trim())) {
    fields.push({ field: "baseUrl", message: "要以 http:// 或 https:// 开头，如 https://api.openai.com/v1。" });
  }
  if (!draft.model.trim()) fields.push({ field: "model", message: "填写接口里的模型名称。" });
  if (fields.length > 0) fail("invalid_config", "模型配置有误", fields);
}

export function createMockLlm(wait: (ms?: number) => Promise<unknown>): LlmApi {
  let settings: LlmSettings = { models: [SAMPLE_MODEL], active: SAMPLE_MODEL.id };
  const listeners = new Set<(delta: ChatDelta) => void>();
  const running = new Map<string, { cancelled: boolean }>();

  const answerFor = (request: ChatRequest) => {
    const last = request.messages.at(-1)?.content ?? "";
    return last.includes("优化") ? OPTIMIZED : GENERATED;
  };

  return {
    async listLlmModels() {
      await wait();
      return structuredClone(settings);
    },
    async saveLlmModel(draft) {
      await wait();
      check(draft);
      const existing = draft.id ? settings.models.find((m) => m.id === draft.id) : undefined;
      if (draft.id && !existing) fail("not_found", `找不到模型：${draft.id}`);
      const hasApiKey = draft.apiKey === null ? (existing?.hasApiKey ?? false) : draft.apiKey.trim() !== "";
      const model: LlmModel = {
        id: existing?.id ?? crypto.randomUUID(),
        name: draft.name.trim(),
        protocol: draft.protocol,
        baseUrl: draft.baseUrl.trim(),
        model: draft.model.trim(),
        maxTokens: draft.maxTokens,
        temperature: draft.temperature,
        hasApiKey,
      };
      const models = existing
        ? settings.models.map((m) => (m.id === model.id ? model : m))
        : [...settings.models, model];
      settings = { models, active: settings.active ?? model.id };
      return model;
    },
    async deleteLlmModel(id) {
      await wait();
      const models = settings.models.filter((m) => m.id !== id);
      settings = { models, active: settings.active === id ? (models[0]?.id ?? null) : settings.active };
      return structuredClone(settings);
    },
    async setActiveLlmModel(id) {
      await wait();
      if (!settings.models.some((m) => m.id === id)) fail("not_found", `找不到模型：${id}`);
      settings = { ...settings, active: id };
      return structuredClone(settings);
    },
    async testLlmModel(draft) {
      await wait(120);
      check(draft);
      if (draft.baseUrl.includes("unreachable")) fail("llm", "模型请求失败：连不上服务器");
      return { reply: "OK", millis: 420 };
    },
    async llmChat(requestId, request) {
      if (!settings.models.some((m) => m.id === request.modelId)) {
        fail("not_found", "找不到这个模型，可能已经删掉了。请在助手里重新选一个。");
      }
      const job = { cancelled: false };
      running.set(requestId, job);
      const answer = answerFor(request);
      try {
        for (let at = 0; at < answer.length; at += CHUNK) {
          await wait(15);
          if (job.cancelled) fail("cancelled", "已取消");
          const delta = { requestId, text: answer.slice(at, at + CHUNK) };
          listeners.forEach((listener) => listener(delta));
        }
      } finally {
        running.delete(requestId);
      }
      return { chars: answer.length, text: answer };
    },
    cancelLlmChat(requestId) {
      const job = running.get(requestId);
      if (job) job.cancelled = true;
      return Promise.resolve(job !== undefined);
    },
    onLlmChat(handler) {
      listeners.add(handler);
      return Promise.resolve(() => void listeners.delete(handler));
    },
  };
}
