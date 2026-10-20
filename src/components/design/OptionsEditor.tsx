import type { TableDesign, TableOptions } from "../../lib/designTypes";
import type { DriverKind } from "../../lib/types";
import { Combo } from "./Combo";
import * as model from "./designModel";

interface OptionsEditorProps {
  driver: DriverKind;
  draft: TableDesign;
  onChange(next: TableDesign): void;
}

const ENGINES = ["InnoDB", "MyISAM", "MEMORY", "ARCHIVE", "CSV", "Aria"];
const CHARSETS = ["utf8mb4", "utf8mb3", "latin1", "gbk", "gb18030", "ascii", "binary"];

/** 选项页:表注释,MySQL 的引擎、字符集、排序规则;另外列出只读的检查约束和触发器。 */
export function OptionsEditor({ driver, draft, onChange }: OptionsEditorProps) {
  const options = draft.options;
  const set = (patch: Partial<TableOptions>) => onChange(model.updateOptions(draft, patch));
  const text = (value: string) => (value.trim() === "" ? null : value);
  const mysql = model.isMysql(driver);
  return (
    <div className="designer-pane">
      <div className="options-form">
        {mysql && (
          <>
            <OptionRow label="存储引擎">
              <Combo
                label="存储引擎"
                value={options.engine ?? ""}
                options={ENGINES}
                placeholder="服务器默认"
                onChange={(engine) => set({ engine: text(engine) })}
              />
            </OptionRow>
            <OptionRow label="字符集">
              <Combo
                label="字符集"
                value={options.charset ?? ""}
                options={CHARSETS}
                placeholder="跟随库"
                onChange={(charset) =>
                  // 换了字符集,不属于它的排序规则就不能留着,否则 MySQL 会报错。
                  set({
                    charset: text(charset),
                    collation: options.collation?.startsWith(`${charset}_`) ? options.collation : null,
                  })
                }
              />
            </OptionRow>
            <OptionRow label="排序规则" hint="只改表的默认值，已有字段不受影响。">
              <input
                className="design-input mono"
                aria-label="排序规则"
                value={options.collation ?? ""}
                placeholder="跟随字符集"
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                onChange={(event) => set({ collation: text(event.target.value) })}
              />
            </OptionRow>
          </>
        )}
        {driver !== "sqlite" && (
          <OptionRow label="注释">
            <textarea
              className="design-input design-textarea"
              aria-label="表注释"
              rows={3}
              value={options.comment ?? ""}
              placeholder="这张表是做什么的"
              onChange={(event) => set({ comment: event.target.value === "" ? null : event.target.value })}
            />
          </OptionRow>
        )}
        {driver === "sqlite" && (
          <OptionRow label="表类型" hint="SQLite 建好之后改不了，重建表时保留。">
            <span className="design-flags">
              <span className={options.withoutRowid ? "design-badge" : "design-badge is-off"}>WITHOUT ROWID</span>
              <span className={options.strict ? "design-badge" : "design-badge is-off"}>STRICT</span>
            </span>
          </OptionRow>
        )}
        {draft.primaryKeyName && (
          <OptionRow label="主键约束名">
            <span className="mono selectable">{draft.primaryKeyName}</span>
          </OptionRow>
        )}
      </div>
      <ReadOnlyList
        title="检查约束"
        note="这里不能改，保存时原样保留。"
        items={draft.checks.map((c) => ({ key: c.id, name: c.name, code: c.expression }))}
      />
      <ReadOnlyList
        title="触发器"
        note="重建表时会按原样重建。"
        items={draft.triggers.map((t) => ({ key: t.name, name: t.name, code: t.sql }))}
      />
    </div>
  );
}

function OptionRow({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="option-row">
      <span className="option-label">{label}</span>
      <div className="option-control">
        {children}
        {hint && <p className="option-hint">{hint}</p>}
      </div>
    </div>
  );
}

function ReadOnlyList({
  title,
  note,
  items,
}: {
  title: string;
  note: string;
  items: { key: string; name: string | null; code: string }[];
}) {
  if (items.length === 0) return null;
  return (
    <section className="readonly-list" aria-label={title}>
      <h3 className="readonly-title">
        {title}
        <span className="readonly-note">{note}</span>
      </h3>
      <ul>
        {items.map((item) => (
          <li key={item.key}>
            {item.name && <span className="readonly-name mono">{item.name}</span>}
            <code className="mono selectable">{item.code}</code>
          </li>
        ))}
      </ul>
    </section>
  );
}
