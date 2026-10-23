import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { hasPrimaryModifier, shortcut } from "../../lib/platform";
import { commandHint, completeCommand } from "./redisCommands";

/** 输入框最多长到几行,再多就在框里滚。 */
const MAX_ROWS = 6;

interface ConsoleInputProps {
  /** 每变一次就把焦点放回输入框(点了输出区的空白处)。 */
  focusRequest: number;
  prompt: string;
  running: boolean;
  /** 标签页切过来时把焦点放回输入框。 */
  active: boolean;
  /** 服务器认识的命令名(补全用);还没取到是空的。 */
  serverNames: readonly string[];
  history: readonly string[];
  onRun(text: string): void;
  onCancel(): void;
  onClear(): void;
}

/**
 * 命令行底部的输入框:回车执行(⇧回车换行,粘贴的多行依次执行),↑↓ 翻历史,
 * 打命令名时列出补全(Tab 或回车选中),之后显示这条命令的写法。
 */
export function ConsoleInput(props: ConsoleInputProps) {
  const { history, running } = props;
  const [draft, setDraft] = useState("");
  /** 正在看历史里的第几条(从旧到新的下标);null 是在编辑自己的草稿。 */
  const [browsing, setBrowsing] = useState<number | null>(null);
  const [ownDraft, setOwnDraft] = useState("");
  const [pick, setPick] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const listId = useId();

  useEffect(() => {
    if (props.active) ref.current?.focus();
  }, [props.active, props.focusRequest]);

  // 打的还是第一个词:补全命令名。完全打对了(唯一的候选就是它)就不再列。
  const completing = !dismissed && /^\S+$/.test(draft);
  const found = completing ? completeCommand(draft, props.serverNames) : [];
  const suggestions = found.length === 1 && found[0] === draft.toUpperCase() ? [] : found;
  const picked = suggestions.length ? Math.min(pick, suggestions.length - 1) : -1;
  const name = /^\s*(\S+)\s/.exec(draft)?.[1] ?? (suggestions.length === 0 ? draft.trim() : "");
  const hint = name && !draft.includes("\n") ? commandHint(name) : null;

  const edit = (text: string) => {
    setDraft(text);
    setBrowsing(null);
    setPick(0);
    setDismissed(false);
  };

  const accept = (command: string) => edit(`${command} `);

  const recall = (index: number | null) => {
    if (browsing === null) setOwnDraft(draft);
    setBrowsing(index);
    setDraft(index === null ? ownDraft : history[index]!);
    setDismissed(true);
  };

  const run = () => {
    if (running || !draft.trim()) return;
    props.onRun(draft);
    edit("");
  };

  const caretOnEdge = (edge: "first" | "last") => {
    const area = ref.current;
    if (!area) return true;
    const before = area.value.slice(0, area.selectionStart);
    return edge === "first" ? !before.includes("\n") : !area.value.slice(area.selectionEnd).includes("\n");
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    const open = suggestions.length > 0;
    if (event.key === "Tab" && open) {
      event.preventDefault();
      accept(suggestions[Math.max(picked, 0)]!);
    } else if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (open && picked >= 0) accept(suggestions[picked]!);
      else run();
    } else if (event.key === "ArrowUp" && open) {
      event.preventDefault();
      setPick((picked - 1 + suggestions.length) % suggestions.length);
    } else if (event.key === "ArrowDown" && open) {
      event.preventDefault();
      setPick((picked + 1) % suggestions.length);
    } else if (event.key === "ArrowUp" && caretOnEdge("first") && history.length > 0) {
      event.preventDefault();
      recall(browsing === null ? history.length - 1 : Math.max(browsing - 1, 0));
    } else if (event.key === "ArrowDown" && caretOnEdge("last") && browsing !== null) {
      event.preventDefault();
      recall(browsing + 1 < history.length ? browsing + 1 : null);
    } else if (event.key === "Escape") {
      if (open) setDismissed(true);
      else if (running) props.onCancel();
    } else if (event.ctrlKey && !event.metaKey && event.key === "c" && !hasSelection(event.currentTarget)) {
      // 和终端一样:⌃C 停下正在等的命令,没在等就清掉输入。
      event.preventDefault();
      if (running) props.onCancel();
      else edit("");
    } else if ((hasPrimaryModifier(event) && event.key === "k") || (event.ctrlKey && event.key === "l")) {
      event.preventDefault();
      props.onClear();
    }
  };

  const rows = Math.min(draft.split("\n").length, MAX_ROWS);

  return (
    <div className="console-input-area">
      {suggestions.length > 0 && (
        <ul id={listId} className="console-suggest" role="listbox" aria-label="命令补全">
          {suggestions.map((command, i) => {
            const about = commandHint(command);
            return (
              <li
                key={command}
                id={`${listId}-${i}`}
                role="option"
                aria-selected={i === picked}
                className={i === picked ? "is-picked" : undefined}
                onMouseDown={(event) => {
                  event.preventDefault();
                  accept(command);
                }}
              >
                <span className="console-suggest-name mono">{command}</span>
                {about && <span className="console-suggest-args mono">{about.args}</span>}
                {about && <span className="console-suggest-summary">{about.summary}</span>}
              </li>
            );
          })}
        </ul>
      )}
      <div className={running ? "console-input is-running" : "console-input"}>
        <span className="console-prompt mono" aria-hidden>
          {props.prompt}
        </span>
        <textarea
          ref={ref}
          className="mono"
          rows={rows}
          value={draft}
          aria-label="Redis 命令"
          aria-autocomplete="list"
          aria-controls={suggestions.length ? listId : undefined}
          aria-activedescendant={picked >= 0 ? `${listId}-${picked}` : undefined}
          placeholder={running ? "正在等回复… Esc 停下" : "输入命令，回车执行"}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          onChange={(event) => edit(event.target.value)}
          onKeyDown={onKeyDown}
        />
      </div>
      <p className="console-hint" aria-live="polite">
        {hint ? (
          <>
            <span className="mono console-hint-usage">
              {name.toUpperCase()} {hint.args}
            </span>
            <span>{hint.summary}</span>
          </>
        ) : (
          <span>↑↓ 历史 · Tab 补全 · ⇧回车 换行 · Esc 停下 · {shortcut("K")} 清屏</span>
        )}
      </p>
    </div>
  );
}

function hasSelection(area: HTMLTextAreaElement): boolean {
  return area.selectionStart !== area.selectionEnd;
}
