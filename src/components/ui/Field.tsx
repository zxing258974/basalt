import { useId, type InputHTMLAttributes, type ReactNode } from "react";

interface FieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "onChange"> {
  label: string;
  error?: string;
  hint?: ReactNode;
  /** 输入框右侧的附加控件,如「浏览…」按钮。 */
  trailing?: ReactNode;
  onValueChange(value: string): void;
}

export function Field({ label, error, hint, trailing, onValueChange, className, ...input }: FieldProps) {
  const id = useId();
  const messageId = `${id}-message`;
  const message = error ?? hint;
  return (
    <div className={["field", error && "has-error", className].filter(Boolean).join(" ")}>
      <label htmlFor={id} className="field-label">
        {label}
      </label>
      <div className="field-control">
        <input
          {...input}
          id={id}
          className="field-input"
          aria-invalid={Boolean(error)}
          aria-describedby={message ? messageId : undefined}
          onChange={(event) => onValueChange(event.target.value)}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
        />
        {trailing}
      </div>
      {message && (
        <p id={messageId} className={error ? "field-error" : "field-hint"}>
          {message}
        </p>
      )}
    </div>
  );
}

interface CheckboxProps {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange(checked: boolean): void;
}

export function Checkbox({ label, checked, disabled, onCheckedChange }: CheckboxProps) {
  return (
    <label className={disabled ? "checkbox is-disabled" : "checkbox"}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onCheckedChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

interface SelectFieldProps<T extends string> {
  label: string;
  value: T;
  options: readonly { value: T; label: string; disabled?: boolean }[];
  error?: string;
  hint?: ReactNode;
  disabled?: boolean;
  className?: string;
  onValueChange(value: T): void;
}

/** 和 Field 一样排版的下拉框。 */
export function SelectField<T extends string>({
  label,
  value,
  options,
  error,
  hint,
  disabled,
  className,
  onValueChange,
}: SelectFieldProps<T>) {
  const id = useId();
  const messageId = `${id}-message`;
  const message = error ?? hint;
  return (
    <div className={["field", error && "has-error", className].filter(Boolean).join(" ")}>
      <label htmlFor={id} className="field-label">
        {label}
      </label>
      <div className="field-control">
        <select
          id={id}
          className="select field-select"
          value={value}
          disabled={disabled}
          aria-invalid={Boolean(error)}
          aria-describedby={message ? messageId : undefined}
          onChange={(event) => onValueChange(event.target.value as T)}
        >
          {options.map((option) => (
            <option key={option.value} value={option.value} disabled={option.disabled}>
              {option.label}
            </option>
          ))}
        </select>
      </div>
      {message && (
        <p id={messageId} className={error ? "field-error" : "field-hint"}>
          {message}
        </p>
      )}
    </div>
  );
}
