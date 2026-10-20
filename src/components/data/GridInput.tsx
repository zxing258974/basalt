import { useRef, useState, type CSSProperties, type KeyboardEvent, type Ref } from "react";

/** 不经组字直接上屏的输入:普通键入,以及拼音模式下的中文标点这类输入法直接给出的字。 */
const TYPED_INPUT = new Set(["insertText", "insertReplacementText"]);

interface GridInputProps {
  ref: Ref<HTMLTextAreaElement>;
  label: string;
  /** 表格(role=grid)的 id 和当前格的 id;读屏从这里读到焦点所在的格子。 */
  controls: string;
  activeCell: string | undefined;
  /**
   * 选中格能不能改。不能改时照样接键盘、照样让输入法组字(只是不显示组字框),打出来的字由表格丢掉;
   * 不用 readOnly:焦点不动时切换 readOnly,WKWebView 不一定重新判断要不要把按键交给输入法。
   */
  editable: boolean;
  /** 盖在选中格上的位置,和编辑框打开时一样:输入法候选窗跟着出现在格子旁。 */
  style: CSSProperties;
  onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): void;
  onCompositionStart(): void;
  /** 打出来的字,作为编辑框的初始内容;composed 表示来自组字结束。 */
  onText(text: string, composed: boolean): void;
}

/**
 * 表格的键盘焦点:一个看不见的 textarea。表格本身不是可编辑元素,输入法不会介入,
 * 所以键盘焦点放在这里——方向键、快捷键交给表格处理,打字和组字由浏览器照常进行,
 * 文字上屏(非组字的输入、或组字结束)后才交给 onText 打开编辑框。
 * 组字过程中这个框显示成编辑框的样子,好看到拼音;粘贴、拖放进来的内容一律不收。
 */
export function GridInput({
  ref,
  label,
  controls,
  activeCell,
  editable,
  style,
  onKeyDown,
  onCompositionStart,
  onText,
}: GridInputProps) {
  const composing = useRef(false);
  const [visible, setVisible] = useState(false);

  // 只从框里取一次:不同浏览器 compositionend 和最后一个 input 的先后不同,先到的取走,后到的拿到空串。
  const take = (input: HTMLTextAreaElement, composed: boolean, fallback = "") => {
    const text = input.value || fallback;
    input.value = "";
    if (text) onText(text, composed);
  };

  return (
    <textarea
      ref={ref}
      className={visible && editable ? "grid-input is-composing" : "grid-input"}
      aria-label={label}
      aria-controls={controls}
      aria-activedescendant={activeCell}
      rows={1}
      spellCheck={false}
      autoCorrect="off"
      autoCapitalize="off"
      style={style}
      onKeyDown={onKeyDown}
      onCompositionStart={() => {
        composing.current = true;
        setVisible(true);
        onCompositionStart();
      }}
      onCompositionEnd={(event) => {
        composing.current = false;
        setVisible(false);
        take(event.currentTarget, true, event.data);
      }}
      onInput={(event) => {
        const { inputType, isComposing } = event.nativeEvent;
        if (composing.current || isComposing) return;
        if (TYPED_INPUT.has(inputType)) take(event.currentTarget, false);
        else event.currentTarget.value = "";
      }}
      // 焦点被对话框等拿走时组字不一定被取消:状态复位、残字清掉,回来时从干净的框开始。
      onBlur={(event) => {
        composing.current = false;
        setVisible(false);
        event.currentTarget.value = "";
      }}
      onPaste={(event) => event.preventDefault()}
      onDrop={(event) => event.preventDefault()}
    />
  );
}
