import { useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import type { EditInput } from "../../lib/types";
import { hasPrimaryModifier } from "../../lib/platform";

/** 提交后选区往哪移。 */
export type EditMove = "stay" | "right" | "left";

interface CellEditorProps {
  /** 打开时的值;null 表示原值是 NULL,编辑框里显示为空。 */
  initial: EditInput;
  /** 打字直接开始编辑时,用这个字符替换原内容。 */
  seed?: string;
  label: string;
  onCommit(value: string, move: EditMove): void;
  onCancel(): void;
}

const MAX_VISIBLE_LINES = 8;

/**
 * 用 textarea 而不是 input:input 会悄悄吞掉换行,编辑多行文本就把数据改了。
 * 回车提交,Shift / Option + 回车换行,Esc 取消;输入法组字时的回车不算提交。
 * 没动过内容就关掉时不算修改,原来的 NULL 不会变成空字符串。
 */
export function CellEditor({ initial, seed, label, onCommit, onCancel }: CellEditorProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState(seed ?? initial ?? "");
  const dirty = useRef(seed !== undefined);
  const finished = useRef(false);

  useLayoutEffect(() => {
    const editor = ref.current;
    if (!editor) return;
    editor.focus();
    const end = editor.value.length;
    editor.setSelectionRange(seed === undefined ? 0 : end, end);
  }, [seed]);

  const finish = (move: EditMove | null) => {
    if (finished.current) return;
    finished.current = true;
    if (move === null || !dirty.current) onCancel();
    else onCommit(text, move);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // ⌘S:先提交这一格,事件继续冒泡,由表格页打开提交预览。
    if (hasPrimaryModifier(event) && event.key.toLowerCase() === "s") {
      finish("stay");
      return;
    }
    event.stopPropagation();
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (event.key === "Escape") {
      event.preventDefault();
      finish(null);
    } else if (event.key === "Enter" && !event.shiftKey && !event.altKey) {
      event.preventDefault();
      finish("stay");
    } else if (event.key === "Tab") {
      event.preventDefault();
      finish(event.shiftKey ? "left" : "right");
    }
  };

  const lines = Math.min(MAX_VISIBLE_LINES, text.split("\n").length);
  return (
    <textarea
      ref={ref}
      className={lines > 1 ? "cell-editor is-multiline" : "cell-editor"}
      aria-label={label}
      value={text}
      rows={lines}
      spellCheck={false}
      autoCorrect="off"
      autoCapitalize="off"
      onChange={(event) => {
        dirty.current = true;
        setText(event.target.value);
      }}
      onKeyDown={onKeyDown}
      onBlur={() => finish("stay")}
    />
  );
}
