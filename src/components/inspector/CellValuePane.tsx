import { Binary, Braces, CircleSlash, Copy, Type } from "lucide-react";
import { memo, useMemo, useState } from "react";
import { useCellPeek, type CellPeek } from "../../stores/cellPeek";
import { useTabs } from "../../stores/tabs";
import { Button } from "../ui/Button";
import { jsonTokens, prettyJson, textStats } from "./cellPeekModel";

/** 框里最多先放这么多字:几 MB 的长文本、BLOB 整段塞进去,WebKit 排版会卡住。复制的总是全文。 */
const MAX_SHOWN_CHARS = 200_000;

const UNSET_TEXT: Record<string, string> = {
  自动生成: "新增行还没填这一格，提交后由数据库生成。",
  默认: "新增行还没填这一格，提交后用列的默认值。",
  NULL: "新增行还没填这一格，提交后是 NULL。",
  必填: "新增行还没填这一格，这一格必填。",
};

/** 信息面板「值」页:当前标签页的表格里选中的那一格的完整内容。 */
export function CellValuePane() {
  const activeId = useTabs((s) => s.activeId);
  const peek = useCellPeek((s) => (s.tabId !== null && s.tabId === activeId ? s.peek : null));
  // 原文 / 格式化在换格子时保留:看一列 JSON 时不用每格都点一次。
  const [raw, setRaw] = useState(false);
  return (
    <div className="inspector-body" role="tabpanel" aria-label="值">
      {peek ? (
        <PeekView peek={peek} raw={raw} onRawChange={setRaw} />
      ) : (
        <p className="inspector-empty">在表格里选中一格，这里显示它的完整内容。</p>
      )}
    </div>
  );
}

interface PeekViewProps {
  peek: CellPeek;
  raw: boolean;
  onRawChange(raw: boolean): void;
}

/** memo:拖面板宽度时整个应用跟着重新渲染,内容没变就不重排、不重新上色。 */
const PeekView = memo(function PeekView({ peek, raw, onRawChange }: PeekViewProps) {
  const { value } = peek;
  const text = value.kind === "text" ? value.text : null;
  const hex = value.kind === "bytes" ? value.hex : null;
  const original = useMemo(() => text ?? (hex === null ? null : `0x${hex}`), [hex, text]);
  const pretty = useMemo(() => (text === null ? null : prettyJson(text)), [text]);
  const shown = pretty !== null && !raw ? pretty : original;
  const [copied, setCopied] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const clipped = shown !== null && shown.length > MAX_SHOWN_CHARS && expanded !== shown;
  const visible = shown !== null && clipped ? clip(shown) : shown;
  const stats = useMemo(() => describeLength(text, hex), [hex, text]);
  const where = [peek.dataType, peek.source, peek.rowNumber === null ? "新增行" : `第 ${peek.rowNumber} 行`];
  const copy = (full: string) => {
    void navigator.clipboard
      ?.writeText(full)
      .then(() => setCopied(full))
      .catch((error: unknown) => console.warn("复制单元格内容失败", error));
  };
  return (
    <>
      <header className="inspector-head">
        <span className="inspector-icon" aria-hidden>
          {kindIcon(peek, pretty !== null)}
        </span>
        <div className="inspector-title">
          <h2 className="inspector-name selectable">{peek.column}</h2>
          <p className="inspector-where">{where.join(" · ")}</p>
        </div>
      </header>
      <div className="inspector-value-wrap">
        <div className="inspector-value-bar">
          {peek.deleted && <span className="inspector-value-badge is-deleted">将删除</span>}
          {peek.edited && <span className="inspector-value-badge">未提交</span>}
          {pretty !== null && (
            <div className="inspector-value-mode" role="group" aria-label="JSON 显示方式">
              <button type="button" aria-pressed={!raw} onClick={() => onRawChange(false)}>
                格式化
              </button>
              <button type="button" aria-pressed={raw} onClick={() => onRawChange(true)}>
                原文
              </button>
            </div>
          )}
          {shown !== null && shown !== "" && (
            <Button
              size="sm"
              variant="ghost"
              icon={<Copy />}
              className="inspector-value-copy"
              onClick={() => copy(shown)}
            >
              {copied === shown ? "已复制" : "复制"}
            </Button>
          )}
          <span className="visually-hidden" role="status">
            {copied !== null && copied === shown ? "已复制到剪贴板" : ""}
          </span>
        </div>
        {visible === null ? (
          <p className="inspector-value-note">{noteFor(peek)}</p>
        ) : visible === "" ? (
          <p className="inspector-value-note">空字符串</p>
        ) : (
          // 能拿到焦点:只用键盘也能上下滚动长内容。
          <pre className="inspector-value mono selectable" aria-label="单元格内容" tabIndex={0}>
            {pretty !== null && !raw && visible.length <= MAX_SHOWN_CHARS ? <ColoredJson text={visible} /> : visible}
          </pre>
        )}
        {clipped && (
          <p className="inspector-value-clipped">
            <span>只显示了前 {MAX_SHOWN_CHARS.toLocaleString()} 个字符，复制的是全部。</span>
            <Button size="sm" variant="ghost" onClick={() => setExpanded(shown)}>
              显示全部
            </Button>
          </p>
        )}
        {stats && <p className="inspector-value-stats">{stats}</p>}
      </div>
    </>
  );
});

export const ColoredJson = memo(function ColoredJson({ text }: { text: string }) {
  const tokens = useMemo(() => jsonTokens(text), [text]);
  return tokens.map((token, index) =>
    token.kind === "space" || token.kind === "punct" ? (
      token.text
    ) : (
      <span key={index} className={`json-${token.kind}`}>
        {token.text}
      </span>
    ),
  );
});

/** 截到 MAX_SHOWN_CHARS,不把一个 emoji(两个 UTF-16 单元)切成两半。 */
function clip(text: string): string {
  const end = /[\uD800-\uDBFF]/.test(text[MAX_SHOWN_CHARS - 1] ?? "") ? MAX_SHOWN_CHARS - 1 : MAX_SHOWN_CHARS;
  return text.slice(0, end);
}

function kindIcon(peek: CellPeek, isJson: boolean) {
  if (isJson) return <Braces />;
  switch (peek.value.kind) {
    case "bytes":
      return <Binary />;
    case "null":
    case "unsupported":
    case "unset":
      return <CircleSlash />;
    case "text":
      return <Type />;
  }
}

function noteFor(peek: CellPeek): string {
  switch (peek.value.kind) {
    case "null":
      return "NULL：这一格没有值（不是空字符串）。";
    case "unsupported":
      return `Basalt 显示不了 ${peek.value.typeName} 类型的值。可以在 SQL 编辑器里把它转成文本再查。`;
    case "unset":
      return UNSET_TEXT[peek.value.hint] ?? `新增行还没填这一格：${peek.value.hint}。`;
    default:
      return "";
  }
}

/** 值本身有多长(不是排版后的):字节、或字符和行数。 */
function describeLength(text: string | null, hex: string | null): string | null {
  if (hex !== null) return `${(hex.length / 2).toLocaleString()} 字节`;
  if (text === null || text === "") return null;
  const { chars, lines } = textStats(text);
  return lines > 1 ? `${chars.toLocaleString()} 字符 · ${lines.toLocaleString()} 行` : `${chars.toLocaleString()} 字符`;
}
