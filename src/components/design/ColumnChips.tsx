import { ArrowDownNarrowWide, ArrowUpNarrowWide, X } from "lucide-react";

export interface Chip {
  /** 字段 id 或字段名,看用在哪里。 */
  value: string;
  label: string;
  descending?: boolean;
}

interface ColumnChipsProps {
  label: string;
  chips: Chip[];
  /** 还能加进来的选项。 */
  options: { value: string; label: string }[];
  onAdd(value: string): void;
  onRemove(index: number): void;
  /** 有它才显示升降序切换(索引用,外键不用)。 */
  onToggleOrder?(index: number): void;
  emptyHint?: string;
}

/** 有顺序的字段列表:一个个小标签,末尾一个下拉框往里加。 */
export function ColumnChips({ label, chips, options, onAdd, onRemove, onToggleOrder, emptyHint }: ColumnChipsProps) {
  return (
    <div className="chips" role="group" aria-label={label}>
      {chips.length === 0 && emptyHint && <span className="chips-empty">{emptyHint}</span>}
      {chips.map((chip, index) => (
        <span key={`${chip.value}-${index}`} className="chip">
          <span className="chip-label mono">{chip.label}</span>
          {onToggleOrder && (
            <button
              type="button"
              className="chip-button"
              aria-label={`${chip.label}：${chip.descending ? "改成升序" : "改成降序"}`}
              title={chip.descending ? "降序" : "升序"}
              onClick={() => onToggleOrder(index)}
            >
              {chip.descending ? <ArrowDownNarrowWide aria-hidden /> : <ArrowUpNarrowWide aria-hidden />}
            </button>
          )}
          <button
            type="button"
            className="chip-button"
            aria-label={`移除 ${chip.label}`}
            onClick={() => onRemove(index)}
          >
            <X aria-hidden />
          </button>
        </span>
      ))}
      {options.length > 0 && (
        <select
          className="chips-add"
          aria-label={`${label}：添加字段`}
          value=""
          onChange={(event) => {
            if (event.target.value) onAdd(event.target.value);
          }}
        >
          <option value="">添加字段…</option>
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      )}
    </div>
  );
}
