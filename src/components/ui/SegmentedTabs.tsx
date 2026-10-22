import type { KeyboardEvent } from "react";

export interface SegmentedTab<T extends string> {
  id: T;
  label: string;
  /** 标签右边的小圆点:这一页有改动(on),或者有字段出错(error)。 */
  mark?: "on" | "error" | null;
}

interface SegmentedTabsProps<T extends string> {
  tabs: readonly SegmentedTab<T>[];
  selected: T;
  onSelect(id: T): void;
  label: string;
  /** 标签与面板的 id 前缀,面板用 tabPanelProps 拿到对应的属性。 */
  prefix: string;
}

const MARK_TEXT = { on: "（已设置）", error: "（有错误）" } as const;

export const tabElementId = (prefix: string, id: string) => `${prefix}-tab-${id}`;

export function tabPanelProps(prefix: string, selected: string) {
  return {
    role: "tabpanel" as const,
    id: `${prefix}-panel`,
    "aria-labelledby": tabElementId(prefix, selected),
  };
}

/** 分段式页签:选中的那段浮起来,左右方向键切换(焦点跟着走)。 */
export function SegmentedTabs<T extends string>({ tabs, selected, onSelect, label, prefix }: SegmentedTabsProps<T>) {
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const index = tabs.findIndex((t) => t.id === selected);
    const step = event.key === "ArrowRight" ? 1 : -1;
    const next = tabs[(index + step + tabs.length) % tabs.length]!;
    onSelect(next.id);
    document.getElementById(tabElementId(prefix, next.id))?.focus();
  };
  return (
    <div className="seg-tabs" role="tablist" aria-label={label} onKeyDown={onKeyDown}>
      {tabs.map((tab) => {
        const isSelected = tab.id === selected;
        return (
          <button
            key={tab.id}
            id={tabElementId(prefix, tab.id)}
            type="button"
            role="tab"
            className="seg-tab"
            aria-selected={isSelected}
            aria-controls={`${prefix}-panel`}
            tabIndex={isSelected ? 0 : -1}
            onClick={() => onSelect(tab.id)}
          >
            {tab.label}
            {tab.mark && (
              <>
                <span className={`seg-tab-mark is-${tab.mark}`} aria-hidden />
                {/* 小圆点读屏读不到:用文字说这一页开着,或者有字段出错。 */}
                <span className="visually-hidden">{MARK_TEXT[tab.mark]}</span>
              </>
            )}
          </button>
        );
      })}
    </div>
  );
}
