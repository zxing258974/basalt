// SQL 助手的类型,与 crates/basalt-core 的 llm 模块一一对应。

/** OpenAI 兼容(Chat Completions)或 Claude(Anthropic Messages)。 */
export type LlmProtocol = "openai" | "anthropic";

export interface LlmModel {
  id: string;
  name: string;
  protocol: LlmProtocol;
  baseUrl: string;
  model: string;
  maxTokens: number | null;
  temperature: number | null;
  /** 钥匙串里存了 API Key。 */
  hasApiKey: boolean;
}

export interface LlmModelDraft {
  /** null:新建。 */
  id: string | null;
  name: string;
  protocol: LlmProtocol;
  baseUrl: string;
  model: string;
  maxTokens: number | null;
  temperature: number | null;
  /** null:不改钥匙串里的;"":删掉;其余:换成这个。 */
  apiKey: string | null;
}

export interface LlmSettings {
  models: LlmModel[];
  /** 助手默认用的模型。 */
  active: string | null;
}

export type ChatRole = "user" | "assistant";

export interface ChatMessage {
  role: ChatRole;
  content: string;
}

export interface ChatRequest {
  modelId: string;
  system: string;
  messages: ChatMessage[];
}

/** 流式回答的一段,按请求 id 认领。 */
export interface ChatDelta {
  requestId: string;
  text: string;
}

export interface ChatOutcome {
  chars: number;
  /** 完整的回答:事件和命令的返回不保证先后,以它为准。 */
  text: string;
}

export interface TestReply {
  reply: string;
  millis: number;
}

export interface OutlineColumn {
  name: string;
  dataType: string;
  primaryKey: boolean;
  nullable: boolean;
  comment: string | null;
}

/** 给助手的表结构概要:只有结构,没有数据。 */
export interface OutlineTable {
  name: string;
  isView: boolean;
  comment: string | null;
  columns: OutlineColumn[];
}
