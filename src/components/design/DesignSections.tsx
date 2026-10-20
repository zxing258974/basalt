import { useId, type KeyboardEvent } from "react";
import type { TableDesign } from "../../lib/designTypes";
import type { DriverKind, TableRef } from "../../lib/types";
import { ColumnsEditor } from "./ColumnsEditor";
import { ForeignKeysEditor } from "./ForeignKeysEditor";
import { IndexesEditor } from "./IndexesEditor";
import { OptionsEditor } from "./OptionsEditor";
import { SqlPreviewPane } from "./SqlPreviewPane";

export type DesignSection = "columns" | "indexes" | "foreignKeys" | "options" | "sql";

const SECTIONS: { id: DesignSection; label: string; count?(design: TableDesign): number }[] = [
  { id: "columns", label: "字段", count: (d) => d.columns.length },
  { id: "indexes", label: "索引", count: (d) => d.indexes.length },
  { id: "foreignKeys", label: "外键", count: (d) => d.foreignKeys.length },
  { id: "options", label: "选项" },
  { id: "sql", label: "SQL 预览" },
];

const tabId = (prefix: string, section: DesignSection) => `${prefix}-tab-${section}`;
const panelId = (prefix: string) => `${prefix}-panel`;

interface DesignTabsProps {
  section: DesignSection;
  onSelect(section: DesignSection): void;
  draft: TableDesign | null;
  /** 标签与面板的 id 前缀,两边要一致。 */
  prefix: string;
}

/** 分段式的页签,左右方向键切换。 */
export function DesignTabs({ section, onSelect, draft, prefix }: DesignTabsProps) {
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const index = SECTIONS.findIndex((s) => s.id === section);
    const step = event.key === "ArrowRight" ? 1 : -1;
    const next = SECTIONS[(index + step + SECTIONS.length) % SECTIONS.length]!;
    onSelect(next.id);
    document.getElementById(tabId(prefix, next.id))?.focus();
  };
  return (
    <div className="designer-tabs" role="tablist" aria-label="表结构" onKeyDown={onKeyDown}>
      {SECTIONS.map((s) => {
        const selected = s.id === section;
        const count = draft && s.count ? s.count(draft) : null;
        return (
          <button
            key={s.id}
            id={tabId(prefix, s.id)}
            type="button"
            role="tab"
            className="designer-tab"
            aria-selected={selected}
            aria-controls={panelId(prefix)}
            tabIndex={selected ? 0 : -1}
            onClick={() => onSelect(s.id)}
          >
            {s.label}
            {count !== null && <span className="designer-tab-count mono">{count}</span>}
          </button>
        );
      })}
    </div>
  );
}

export function useSectionPrefix(): string {
  return `design-${useId().replaceAll(":", "")}`;
}

interface DesignEditorsProps {
  section: DesignSection;
  prefix: string;
  driver: DriverKind;
  connectionId: string;
  original: TableDesign | null;
  originalRef: TableRef | null;
  draft: TableDesign;
  onChange(next: TableDesign): void;
  selectedId: string | null;
  onSelect(id: string | null): void;
}

export function DesignEditors(props: DesignEditorsProps) {
  const { section, prefix, driver, draft, onChange } = props;
  return (
    <div className="designer-panel" role="tabpanel" id={panelId(prefix)} aria-labelledby={tabId(prefix, section)}>
      {section === "columns" && (
        <ColumnsEditor
          driver={driver}
          original={props.original}
          draft={draft}
          onChange={onChange}
          selectedId={props.selectedId}
          onSelect={props.onSelect}
        />
      )}
      {section === "indexes" && <IndexesEditor driver={driver} draft={draft} onChange={onChange} />}
      {section === "foreignKeys" && (
        <ForeignKeysEditor connectionId={props.connectionId} draft={draft} onChange={onChange} />
      )}
      {section === "options" && <OptionsEditor driver={driver} draft={draft} onChange={onChange} />}
      {section === "sql" && (
        <SqlPreviewPane connectionId={props.connectionId} original={props.originalRef} draft={draft} />
      )}
    </div>
  );
}
