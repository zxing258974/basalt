import { useRef, type FocusEvent, type KeyboardEvent } from "react";

export interface Choice<T> {
  value: T;
  label: string;
  /** 写在名字右边的语句或关键字。 */
  code?: string;
  detail: string;
}

interface ChoiceListProps<T> {
  label: string;
  choices: readonly Choice<T>[];
  value: T;
  disabled?: boolean;
  onChange(value: T): void;
}

const STEPS: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 };

/**
 * 一组单选的卡片:名字、语句、一行说明。和导出对话框的「范围」同一套样式。
 * 按单选组的习惯:Tab 只停在选中的那张,方向键换选项;焦点从组外进来(如对话框打开时落到第一张)
 * 时挪到选中的那张,免得没选中的卡片带着焦点环、看着像被选中了。
 */
export function ChoiceList<T>({ label, choices, value, disabled, onChange }: ChoiceListProps<T>) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  // 选中后立刻聚焦到新卡片时,本次渲染的 checked 还是旧的:这次聚焦不能被当成「从组外进来」挪回去。
  const selecting = useRef(false);
  const checked = Math.max(
    choices.findIndex((c) => c.value === value),
    0,
  );

  const select = (index: number) => {
    onChange(choices[index]!.value);
    selecting.current = true;
    buttons.current[index]?.focus();
    selecting.current = false;
  };

  const onKeyDown = (event: KeyboardEvent) => {
    const step = STEPS[event.key];
    if (step === undefined) return;
    event.preventDefault();
    select((checked + step + choices.length) % choices.length);
  };

  const onFocus = (index: number) => (event: FocusEvent<HTMLButtonElement>) => {
    if (selecting.current) return;
    const fromInside = event.currentTarget.parentElement?.contains(event.relatedTarget);
    if (index !== checked && !fromInside) buttons.current[checked]?.focus();
  };

  return (
    <div className="transfer-range" role="radiogroup" aria-label={label} onKeyDown={onKeyDown}>
      {choices.map((choice, index) => (
        <button
          key={choice.label}
          ref={(element) => {
            buttons.current[index] = element;
          }}
          type="button"
          role="radio"
          aria-checked={index === checked}
          tabIndex={index === checked ? 0 : -1}
          disabled={disabled}
          className={index === checked ? "transfer-range-option is-checked" : "transfer-range-option"}
          onFocus={onFocus(index)}
          onClick={() => select(index)}
        >
          <span className="transfer-radio" aria-hidden />
          <span className="transfer-range-label">{choice.label}</span>
          {choice.code && <code className="transfer-range-detail">{choice.code}</code>}
          <span className="transfer-range-text">{choice.detail}</span>
        </button>
      ))}
    </div>
  );
}
