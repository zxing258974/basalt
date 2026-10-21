import { ClipboardCopy, CornerDownLeft } from "lucide-react";
import { useState } from "react";
import type { Turn } from "../../stores/assistant";
import { Button } from "../ui/Button";
import { parseAnswer, riskyKeywords, type AnswerPart } from "./assistantModel";

interface AssistantMessagesProps {
  turns: readonly Turn[];
  onInsert(sql: string): void;
}

/** 一个代码块:等宽显示;助手写的 SQL 可以复制或插到编辑器的光标处(不会执行)。 */
function CodeBlock({ part, onInsert }: { part: Extract<AnswerPart, { kind: "code" }>; onInsert?(sql: string): void }) {
  const [copied, setCopied] = useState(false);
  const sql = part.lang === "" || part.lang === "sql";
  // 只提醒助手写的 SQL;用户自己的话里的不用。
  const risky = onInsert && sql ? riskyKeywords(part.code) : [];
  return (
    <figure className="assistant-code">
      <figcaption className="assistant-code-head">
        <span className="assistant-code-lang mono">{part.lang || "sql"}</span>
        {part.done && (
          <span className="assistant-code-actions">
            <Button
              variant="ghost"
              size="sm"
              icon={<ClipboardCopy />}
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(part.code)
                  .then(() => setCopied(true))
                  .catch((error: unknown) => console.warn("复制 SQL 失败", error));
              }}
            >
              {copied ? "已复制" : "复制"}
            </Button>
            {sql && onInsert && (
              <Button
                variant="ghost"
                size="sm"
                icon={<CornerDownLeft />}
                title="插到编辑器的光标处；选中了文字就替换选中的部分。不会执行"
                onClick={() => onInsert(part.code)}
              >
                插入到编辑器
              </Button>
            )}
          </span>
        )}
      </figcaption>
      {risky.length > 0 && (
        <p className="assistant-risk">
          含 {risky.join("、")}：会改动数据或表结构。执行前逐条看清楚，最好先在事务里试。
        </p>
      )}
      <pre className="assistant-code-body mono selectable">
        <code>{part.code}</code>
      </pre>
    </figure>
  );
}

/** `onInsert` 为空时(用户自己的话)只能复制,不给插入。 */
function Parts({ text, onInsert }: { text: string; onInsert?(sql: string): void }) {
  return (
    <>
      {parseAnswer(text).map((part, index) =>
        part.kind === "text" ? (
          <p key={index} className="assistant-text selectable">
            {part.text}
          </p>
        ) : (
          <CodeBlock key={index} part={part} onInsert={onInsert} />
        ),
      )}
    </>
  );
}

/** 对话:用户的话靠右,助手的回答拆成文字和代码块。 */
export function AssistantMessages({ turns, onInsert }: AssistantMessagesProps) {
  return (
    <ol className="assistant-turns" aria-label="对话">
      {turns.map((turn) => (
        <li
          key={turn.id}
          className={`assistant-turn is-${turn.role} is-${turn.status}`}
          aria-busy={turn.status === "streaming"}
        >
          {turn.role === "user" ? (
            <div className="assistant-question">
              <Parts text={turn.content} />
            </div>
          ) : (
            <div className="assistant-answer">
              {turn.content ? (
                <Parts text={turn.content} onInsert={onInsert} />
              ) : (
                turn.status === "streaming" && <p className="assistant-thinking">正在想…</p>
              )}
              {turn.status === "cancelled" && <p className="assistant-meta">已停止。</p>}
              {turn.status === "error" && (
                <p className="assistant-error" role="alert">
                  {turn.error}
                </p>
              )}
            </div>
          )}
        </li>
      ))}
    </ol>
  );
}
