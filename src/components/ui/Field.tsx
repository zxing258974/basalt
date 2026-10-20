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
  onCheckedChange(checked: boolean): void;
}

export function Checkbox({ label, checked, onCheckedChange }: CheckboxProps) {
  return (
    <label className="checkbox">
      <input type="checkbox" checked={checked} onChange={(e) => onCheckedChange(e.target.checked)} />
      <span>{label}</span>
    </label>
  );
}
