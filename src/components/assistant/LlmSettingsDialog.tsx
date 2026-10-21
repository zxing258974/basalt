import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleCheck, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { getBackend } from "../../lib/api";
import type { LlmModel, LlmModelDraft, LlmProtocol, LlmSettings, TestReply } from "../../lib/llmTypes";
import { errorMessage, type CommandError } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Checkbox, Field } from "../ui/Field";
import { secretStoreName } from "../../lib/platform";

export const LLM_SETTINGS_KEY = ["llm", "settings"] as const;

const PROTOCOLS: { value: LlmProtocol; label: string; baseUrl: string; model: string }[] = [
  {
    value: "openai",
    label: "OpenAI 兼容",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1、deepseek-chat、qwen-plus…",
  },
  { value: "anthropic", label: "Claude", baseUrl: "https://api.anthropic.com", model: "claude-sonnet-4-5…" },
];

const EMPTY: LlmModelDraft = {
  id: null,
  name: "",
  protocol: "openai",
  baseUrl: "",
  model: "",
  maxTokens: null,
  temperature: null,
  apiKey: "",
};

function draftOf(model: LlmModel): LlmModelDraft {
  const { hasApiKey: _hasApiKey, ...rest } = model;
  return { ...rest, apiKey: null };
}

/** 空着是 null;不是数字的是 NaN(JSON 里没有 NaN,发出去前要先拦下)。 */
function parseNumber(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : Number.NaN;
}

/** 两个数字框填得不对时的提示;都对时返回空对象。 */
export function numberErrors(maxTokens: string, temperature: string): Record<string, string> {
  const errors: Record<string, string> = {};
  const max = parseNumber(maxTokens);
  if (max !== null && (!Number.isInteger(max) || max <= 0)) errors.maxTokens = "要填正整数，不填用默认值。";
  const t = parseNumber(temperature);
  if (t !== null && Number.isNaN(t)) errors.temperature = "要填 0 到 2 之间的数字。";
  return errors;
}

export function useLlmSettings() {
  return useQuery({
    queryKey: LLM_SETTINGS_KEY,
    queryFn: async () => (await getBackend()).listLlmModels(),
  });
}

/** 配置 SQL 助手用的模型。API Key 进钥匙串,不写进配置文件。 */
export function LlmSettingsDialog({ onClose }: { onClose(): void }) {
  const queryClient = useQueryClient();
  const settings = useLlmSettings();
  const models = settings.data?.models ?? [];
  const [selected, setSelected] = useState<string | null>(null);
  const editing = models.find((m) => m.id === selected) ?? null;
  const [draft, setDraft] = useState<LlmModelDraft>(EMPTY);
  const [numbers, setNumbers] = useState({ maxTokens: "", temperature: "" });
  const [clearKey, setClearKey] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [status, setStatus] = useState<{ tone: "ok" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState<"save" | "test" | "delete" | null>(null);

  const pick = (model: LlmModel | null) => {
    setSelected(model?.id ?? null);
    setDraft(model ? draftOf(model) : EMPTY);
    setNumbers({
      maxTokens: model?.maxTokens?.toString() ?? "",
      temperature: model?.temperature?.toString() ?? "",
    });
    setClearKey(false);
    setErrors({});
    setStatus(null);
  };

  const protocol = PROTOCOLS.find((p) => p.value === draft.protocol)!;
  const current = (): LlmModelDraft => ({
    ...draft,
    maxTokens: parseNumber(numbers.maxTokens),
    temperature: parseNumber(numbers.temperature),
    // 编辑时没填新 Key:不改(或者勾了清除)。
    apiKey: draft.apiKey?.trim() ? draft.apiKey.trim() : editing ? (clearKey ? "" : null) : "",
  });
  const update = <K extends keyof LlmModelDraft>(key: K, value: LlmModelDraft[K]) => {
    setDraft((d) => ({ ...d, [key]: value }));
    setErrors((e) => ({ ...e, [key]: "" }));
  };
  const showError = (error: unknown) => {
    const fields = (error as CommandError).fields ?? [];
    setErrors(Object.fromEntries(fields.map((f) => [f.field, f.message])));
    setStatus({ tone: "error", text: errorMessage(error) });
  };
  const refresh = (next: LlmSettings) => queryClient.setQueryData(LLM_SETTINGS_KEY, next);

  /** 数字框填错了就先在本地拦下。 */
  const checked = (): boolean => {
    const local = numberErrors(numbers.maxTokens, numbers.temperature);
    if (Object.keys(local).length === 0) return true;
    setErrors((e) => ({ ...e, ...local }));
    setStatus({ tone: "error", text: "有几项填得不对，看一下标红的地方。" });
    return false;
  };

  const save = async () => {
    if (!checked()) return;
    setBusy("save");
    setStatus(null);
    try {
      const saved = await (await getBackend()).saveLlmModel(current());
      refresh(await (await getBackend()).listLlmModels());
      pick(saved);
      setStatus({ tone: "ok", text: `已保存「${saved.name}」。` });
    } catch (error) {
      showError(error);
    } finally {
      setBusy(null);
    }
  };

  const test = async () => {
    if (!checked()) return;
    setBusy("test");
    setStatus(null);
    try {
      const reply: TestReply = await (await getBackend()).testLlmModel(current());
      setStatus({ tone: "ok", text: `连通了，${reply.millis} 毫秒。模型回复：${reply.reply || "（空）"}` });
    } catch (error) {
      showError(error);
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!editing) return;
    setBusy("delete");
    try {
      refresh(await (await getBackend()).deleteLlmModel(editing.id));
      pick(null);
    } catch (error) {
      showError(error);
    } finally {
      setBusy(null);
    }
  };

  const makeDefault = async () => {
    if (!editing) return;
    try {
      refresh(await (await getBackend()).setActiveLlmModel(editing.id));
    } catch (error) {
      showError(error);
    }
  };

  return (
    <Dialog
      open
      title="模型设置"
      subtitle="SQL 助手用哪些大模型。支持 OpenAI 兼容接口和 Claude 接口。"
      width={760}
      onClose={onClose}
      footer={
        <>
          {editing && (
            <Button variant="ghost" icon={<Trash2 />} busy={busy === "delete"} onClick={() => void remove()}>
              删除
            </Button>
          )}
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={onClose}>
            关闭
          </Button>
          <Button variant="secondary" busy={busy === "test"} disabled={busy !== null} onClick={() => void test()}>
            测试连接
          </Button>
          <Button variant="primary" busy={busy === "save"} disabled={busy !== null} onClick={() => void save()}>
            保存
          </Button>
        </>
      }
    >
      <div className="llm-settings">
        <nav className="llm-models" aria-label="已配置的模型">
          <ul>
            {models.map((model) => (
              <li key={model.id}>
                <button
                  type="button"
                  className={model.id === selected ? "llm-model is-selected" : "llm-model"}
                  aria-current={model.id === selected}
                  onClick={() => pick(model)}
                >
                  <span className="llm-model-name">{model.name}</span>
                  <span className="llm-model-meta mono">{model.model}</span>
                  {settings.data?.active === model.id && <span className="llm-model-tag">默认</span>}
                </button>
              </li>
            ))}
          </ul>
          <Button variant="ghost" size="sm" icon={<Plus />} onClick={() => pick(null)}>
            添加模型
          </Button>
        </nav>
        <form
          className="llm-form"
          aria-label={editing ? `编辑「${editing.name}」` : "添加模型"}
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <Field
            label="名称"
            value={draft.name}
            placeholder="如 DeepSeek、公司网关"
            error={errors.name || undefined}
            onValueChange={(v) => update("name", v)}
          />
          <div
            className="llm-protocol"
            role="radiogroup"
            aria-label="接口格式"
            onKeyDown={(event) => {
              // 单选组:方向键在选项间移动并选中。
              const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
              if (!step) return;
              event.preventDefault();
              const index = PROTOCOLS.findIndex((p) => p.value === draft.protocol);
              const next = PROTOCOLS[(index + step + PROTOCOLS.length) % PROTOCOLS.length]!;
              update("protocol", next.value);
              event.currentTarget.querySelector<HTMLButtonElement>(`[data-value="${next.value}"]`)?.focus();
            }}
          >
            {PROTOCOLS.map((p) => (
              <button
                key={p.value}
                type="button"
                role="radio"
                data-value={p.value}
                tabIndex={draft.protocol === p.value ? 0 : -1}
                aria-checked={draft.protocol === p.value}
                className={draft.protocol === p.value ? "llm-protocol-option is-checked" : "llm-protocol-option"}
                onClick={() => update("protocol", p.value)}
              >
                {p.label}
              </button>
            ))}
          </div>
          <Field
            label="Base URL"
            value={draft.baseUrl}
            placeholder={protocol.baseUrl}
            spellCheck={false}
            error={errors.baseUrl || undefined}
            hint={
              draft.protocol === "openai"
                ? "会请求 {Base URL}/chat/completions；只填域名时自动补 /v1。"
                : "会请求 {Base URL}/v1/messages。"
            }
            onValueChange={(v) => update("baseUrl", v)}
          />
          <Field
            label="模型名称"
            value={draft.model}
            placeholder={protocol.model}
            spellCheck={false}
            error={errors.model || undefined}
            onValueChange={(v) => update("model", v)}
          />
          <Field
            label="API Key"
            type="password"
            value={draft.apiKey ?? ""}
            placeholder={
              editing?.hasApiKey ? `已存在${secretStoreName()}里，留空不改` : "本地模型（如 Ollama）可以不填"
            }
            autoComplete="off"
            error={errors.apiKey || undefined}
            onValueChange={(v) => update("apiKey", v)}
          />
          {editing?.hasApiKey && !draft.apiKey && (
            <Checkbox label="清除已保存的 API Key" checked={clearKey} onCheckedChange={setClearKey} />
          )}
          <div className="llm-form-row">
            <Field
              label="最多输出 tokens"
              value={numbers.maxTokens}
              placeholder={draft.protocol === "anthropic" ? "默认 4096" : "不填用模型默认"}
              inputMode="numeric"
              error={errors.maxTokens || undefined}
              onValueChange={(v) => {
                setNumbers((n) => ({ ...n, maxTokens: v }));
                setErrors((e) => ({ ...e, maxTokens: "" }));
              }}
            />
            <Field
              label="temperature"
              value={numbers.temperature}
              placeholder="0–2，不填用模型默认"
              inputMode="decimal"
              error={errors.temperature || undefined}
              onValueChange={(v) => {
                setNumbers((n) => ({ ...n, temperature: v }));
                setErrors((e) => ({ ...e, temperature: "" }));
              }}
            />
          </div>
          {editing && settings.data?.active !== editing.id && (
            <Button variant="ghost" size="sm" icon={<CircleCheck />} onClick={() => void makeDefault()}>
              设为默认模型
            </Button>
          )}
          {status && (
            <p className={`llm-status is-${status.tone}`} role={status.tone === "error" ? "alert" : "status"}>
              {status.text}
            </p>
          )}
          <p className="llm-note">
            API Key 存在{secretStoreName()}
            里，不写进配置文件。请求只发往上面填的地址，内容是表结构（表名、列名、类型）和你的问题或
            SQL，不含表里的数据。
          </p>
        </form>
      </div>
    </Dialog>
  );
}
