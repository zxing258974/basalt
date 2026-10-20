import { useState } from "react";

interface NumberFieldProps {
  label: string;
  /** 输入框的读屏名字,比标签多说清是什么数。 */
  ariaLabel: string;
  unit: string;
  value: number;
  min: number;
  max: number;
  onChange(value: number): void;
}

/** 整数输入:打字时只记下草稿(删空了也不立刻改成最小值),离开时才收进范围。 */
export function NumberField({ label, ariaLabel, unit, value, min, max, onChange }: NumberFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const clamp = (n: number) => Math.min(max, Math.max(min, Math.round(n)));
  return (
    <label className="transfer-field">
      <span className="transfer-field-label">{label}</span>
      <span className="transfer-input-with-unit">
        <input
          className="transfer-input mono"
          type="number"
          aria-label={ariaLabel}
          min={min}
          max={max}
          value={draft ?? String(value)}
          onChange={(event) => {
            const text = event.target.value;
            setDraft(text);
            const number = Number(text);
            if (text.trim() !== "" && Number.isFinite(number)) onChange(clamp(number));
          }}
          onBlur={() => setDraft(null)}
        />
        <span>{unit}</span>
      </span>
    </label>
  );
}
