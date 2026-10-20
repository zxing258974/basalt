import { ChevronDown } from "lucide-react";
import { useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";

interface ComboProps {
  value: string;
  onChange(value: string): void;
  /** 下拉建议;输入框照样可以随便填。 */
  options: readonly string[];
  label: string;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
}

interface Anchor {
  left: number;
  top: number;
  width: number;
}

/**
 * 可以手填的下拉框(combobox)。建议列表挂在 body 上、按输入框位置定位,
 * 不会被表格的滚动容器裁掉;打开后只按「打开之后输入的内容」过滤。
 */
export function Combo({ value, onChange, options, label, placeholder, disabled, className }: ComboProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  const [query, setQuery] = useState<string | null>(null);
  const [highlight, setHighlight] = useState(-1);
  const open = anchor !== null;

  const filtered = useMemo(() => {
    const needle = query?.trim().toLowerCase();
    if (!needle) return options;
    return options.filter((o) => o.toLowerCase().includes(needle));
  }, [options, query]);

  const place = () => {
    const rect = inputRef.current?.getBoundingClientRect();
    if (rect) setAnchor({ left: rect.left, top: rect.bottom + 2, width: Math.max(rect.width, 160) });
  };
  const show = () => {
    if (disabled) return;
    place();
    setQuery(null);
    setHighlight(options.indexOf(value));
  };
  const hide = () => {
    setAnchor(null);
    setQuery(null);
  };
  const pick = (option: string) => {
    onChange(option);
    hide();
  };

  // 外面滚动时跟着输入框挪。
  useLayoutEffect(() => {
    if (!open) return;
    const onScroll = (event: Event) => {
      if (!(event.target instanceof Node) || !document.getElementById(listId)?.contains(event.target)) {
        const rect = inputRef.current?.getBoundingClientRect();
        if (rect) setAnchor({ left: rect.left, top: rect.bottom + 2, width: Math.max(rect.width, 160) });
      }
    };
    window.addEventListener("scroll", onScroll, true);
    return () => window.removeEventListener("scroll", onScroll, true);
  }, [open, listId]);

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) return show();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setHighlight((current) => Math.min(Math.max(current + step, 0), filtered.length - 1));
    } else if (event.key === "Enter" && open && filtered[highlight] !== undefined) {
      event.preventDefault();
      pick(filtered[highlight]);
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      event.stopPropagation();
      hide();
    }
  };

  const activeId = open && highlight >= 0 ? `${listId}-${highlight}` : undefined;

  return (
    <span className={["combo", className].filter(Boolean).join(" ")}>
      <input
        ref={inputRef}
        className="combo-input"
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={activeId}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        onChange={(event) => {
          onChange(event.target.value);
          if (!open) show();
          setQuery(event.target.value);
          setHighlight(0);
        }}
        onKeyDown={onKeyDown}
        onBlur={hide}
      />
      <button
        type="button"
        className="combo-toggle"
        tabIndex={-1}
        aria-label={`${label}：展开建议`}
        disabled={disabled}
        onMouseDown={(event) => {
          event.preventDefault();
          inputRef.current?.focus();
          if (open) hide();
          else show();
        }}
      >
        <ChevronDown aria-hidden />
      </button>
      {open &&
        filtered.length > 0 &&
        createPortal(
          <ul
            id={listId}
            role="listbox"
            aria-label={label}
            className="combo-list"
            style={{ left: anchor.left, top: anchor.top, minWidth: anchor.width }}
          >
            {filtered.map((option, index) => (
              <li
                key={option}
                id={`${listId}-${index}`}
                role="option"
                aria-selected={index === highlight}
                className={index === highlight ? "combo-option is-active" : "combo-option"}
                onMouseDown={(event) => {
                  event.preventDefault();
                  pick(option);
                }}
                onMouseEnter={() => setHighlight(index)}
              >
                {option}
              </li>
            ))}
          </ul>,
          document.body,
        )}
    </span>
  );
}
