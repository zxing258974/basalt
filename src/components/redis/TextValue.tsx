import { Pencil } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { getBackend } from "../../lib/api";
import { hasPrimaryModifier, shortcut } from "../../lib/platform";
import { errorMessage } from "../../lib/types";
import { ColoredJson } from "../inspector/CellValuePane";
import { prettyJson, textStats } from "../inspector/cellPeekModel";
import { Button } from "../ui/Button";

/** 排好版的 JSON 超过这么长就不上色。 */
const MAX_COLORED_CHARS = 200_000;

interface TextValueProps {
  connectionId: string;
  db: number;
  keyId: string;
  text: string;
  /** 只读出了前一部分:不能在这里改。 */
  truncated: boolean;
  active: boolean;
  onSaved(): Promise<void>;
  onEditingChange(editing: boolean): void;
}

/** 正在改的:`base` 是点「编辑」时读出来的值,保存时拿它核对服务器上的值有没有被别人改过。 */
interface Draft {
  base: string;
  text: string;
}

/** 字符串的值:看的时候 JSON 排版上色;改的时候是文本框,⌘S 保存,被别人改过就不覆盖。 */
export function TextValue(props: TextValueProps) {
  const { text, truncated } = props;
  const pretty = useMemo(() => prettyJson(text), [text]);
  const [raw, setRaw] = useState(false);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const editing = draft !== null;
  const changed = draft !== null && draft.text !== draft.base;
  // 文本框会把 \r\n、\r 都换成 \n:有回车符的值在这里改会悄悄改掉它们。
  const lockedReason = truncated
    ? "值太长，只读出了前 1 MB，不能在这里改"
    : text.includes("\r")
      ? "值里有回车符（\\r），文本框会把它换成换行，不能在这里改；可以在命令行里用 SET 改"
      : null;
  const { onEditingChange } = props;

  useEffect(() => onEditingChange(changed), [changed, onEditingChange]);
  useEffect(() => () => onEditingChange(false), [onEditingChange]);

  const save = async () => {
    if (draft === null || saving) return;
    if (!changed) {
      setDraft(null);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await (await getBackend()).redisSetString(props.connectionId, props.db, props.keyId, draft.base, draft.text);
      await props.onSaved();
      setDraft(null);
    } catch (failure) {
      setError(errorMessage(failure));
    } finally {
      setSaving(false);
    }
  };

  useEffect(() => {
    if (editing) areaRef.current?.focus();
  }, [editing]);

  // 改着的时候 ⌘S 在这个标签页里哪儿按都算(比如刚关掉过期时间的框,焦点不在文本框里)。
  const saveRef = useRef(save);
  useLayoutEffect(() => {
    saveRef.current = save;
  });
  useEffect(() => {
    if (!editing || !props.active) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (hasPrimaryModifier(event) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void saveRef.current();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [editing, props.active]);

  const shown = pretty !== null && !raw ? pretty : text;
  const stats = textStats(text);

  return (
    <div className="key-value text-value">
      <div className="key-value-bar">
        {editing ? (
          <>
            <span className="key-value-hint">
              {draft.base === text
                ? "改的是原文；保存时核对服务器上的值还是读出来的那个，剩余时间不变。"
                : "开始编辑之后服务器上的值变了，保存会被拒绝；可以取消后重新编辑。"}
            </span>
            <Button variant="secondary" size="sm" disabled={saving} onClick={() => setDraft(null)}>
              取消
            </Button>
            <Button
              variant="primary"
              size="sm"
              busy={saving}
              title={shortcut("S")}
              disabled={!changed}
              onClick={() => void save()}
            >
              保存
            </Button>
          </>
        ) : (
          <>
            {pretty !== null && (
              <div className="inspector-value-mode" role="group" aria-label="JSON 显示方式">
                <button type="button" aria-pressed={!raw} onClick={() => setRaw(false)}>
                  格式化
                </button>
                <button type="button" aria-pressed={raw} onClick={() => setRaw(true)}>
                  原文
                </button>
              </div>
            )}
            <span className="key-value-stats">
              {stats.chars.toLocaleString()} 字符{stats.lines > 1 ? ` · ${stats.lines.toLocaleString()} 行` : ""}
            </span>
            <Button
              variant="secondary"
              size="sm"
              icon={<Pencil />}
              disabled={lockedReason !== null}
              title={lockedReason ?? undefined}
              onClick={() => {
                setError(null);
                setDraft({ base: text, text });
              }}
            >
              编辑
            </Button>
          </>
        )}
      </div>
      {error && (
        <p className="key-value-error" role="alert">
          {error}
        </p>
      )}
      {truncated && <p className="key-value-note">值太长，只读出了前 1 MB。在命令行里用 GETRANGE 看其余的部分。</p>}
      {editing ? (
        <textarea
          ref={areaRef}
          className="text-value-editor mono"
          aria-label="字符串的值"
          value={draft.text}
          spellCheck={false}
          onChange={(event) => setDraft({ ...draft, text: event.target.value })}
          onKeyDown={(event) => {
            if (event.key === "Escape" && !changed) setDraft(null);
          }}
        />
      ) : text === "" ? (
        <p className="key-value-note">空字符串</p>
      ) : (
        <pre className="inspector-value text-value-view mono selectable" aria-label="字符串的值" tabIndex={0}>
          {pretty !== null && !raw && shown.length <= MAX_COLORED_CHARS ? <ColoredJson text={shown} /> : shown}
        </pre>
      )}
    </div>
  );
}
