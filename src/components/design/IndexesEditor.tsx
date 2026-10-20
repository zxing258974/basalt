import { Plus, Trash2 } from "lucide-react";
import type { IndexDef, TableDesign } from "../../lib/designTypes";
import type { DriverKind } from "../../lib/types";
import { Button } from "../ui/Button";
import { ColumnChips } from "./ColumnChips";
import * as model from "./designModel";

interface IndexesEditorProps {
  driver: DriverKind;
  draft: TableDesign;
  onChange(next: TableDesign): void;
}

/** 索引页:每个索引一张卡片,字段按顺序排、可切升降序。 */
export function IndexesEditor({ driver, draft, onChange }: IndexesEditorProps) {
  const methods = model.indexMethods(driver);
  const update = (id: string, patch: Partial<IndexDef>) => onChange(model.updateIndex(draft, id, patch));
  return (
    <div className="designer-pane">
      <div className="designer-toolbar" role="toolbar" aria-label="索引操作">
        <Button size="sm" variant="ghost" icon={<Plus />} onClick={() => onChange(model.addIndex(draft))}>
          新增索引
        </Button>
        <span className="designer-toolbar-hint">主键不在这里，在字段页里勾。</span>
      </div>
      <div className="design-cards">
        {draft.indexes.length === 0 && <p className="designer-empty">这张表还没有索引。</p>}
        {draft.indexes.map((index) => (
          <article key={index.id} className="design-card" aria-label={`索引 ${index.name || "（未命名）"}`}>
            <div className="design-card-head">
              <input
                className="design-card-name mono"
                aria-label="索引名"
                value={index.name}
                placeholder="索引名"
                disabled={index.definition !== null}
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                onChange={(event) => update(index.id, { name: event.target.value })}
              />
              {index.isConstraint && <span className="design-badge">唯一约束</span>}
              <span className="design-card-spacer" />
              {index.definition === null && (
                <>
                  <label className="checkbox">
                    <input
                      type="checkbox"
                      checked={index.unique}
                      onChange={(event) => update(index.id, { unique: event.target.checked })}
                    />
                    <span>唯一</span>
                  </label>
                  {methods.length > 1 && (
                    <select
                      className="design-select"
                      aria-label={`${index.name}：索引类型`}
                      value={index.method ?? ""}
                      onChange={(event) => update(index.id, { method: event.target.value || null })}
                    >
                      {methods.map((method) => (
                        <option key={method ?? ""} value={method ?? ""}>
                          {method ?? "默认（B 树）"}
                        </option>
                      ))}
                    </select>
                  )}
                </>
              )}
              <Button
                size="sm"
                variant="ghost"
                icon={<Trash2 />}
                aria-label={`删除索引 ${index.name}`}
                onClick={() => onChange(model.removeIndex(draft, index.id))}
              />
            </div>
            {index.definition !== null ? (
              <div className="design-raw">
                <code className="mono selectable">{index.definition}</code>
                <p>表达式索引、部分索引这类在这里只能删除，要改请到 SQL 编辑器里写。</p>
              </div>
            ) : (
              <IndexColumns draft={draft} index={index} onChange={(columns) => update(index.id, { columns })} />
            )}
          </article>
        ))}
      </div>
    </div>
  );
}

function IndexColumns({
  draft,
  index,
  onChange,
}: {
  draft: TableDesign;
  index: IndexDef;
  onChange(columns: IndexDef["columns"]): void;
}) {
  const nameOf = (id: string) => draft.columns.find((c) => c.id === id)?.name || "（未命名字段）";
  const used = new Set(index.columns.map((c) => c.column));
  return (
    <ColumnChips
      label={`${index.name || "索引"} 的字段`}
      chips={index.columns.map((c) => ({ value: c.column, label: nameOf(c.column), descending: c.descending }))}
      options={draft.columns.filter((c) => !used.has(c.id)).map((c) => ({ value: c.id, label: nameOf(c.id) }))}
      emptyHint="至少选一个字段"
      onAdd={(column) => onChange([...index.columns, { column, descending: false, length: null }])}
      onRemove={(at) => onChange(index.columns.filter((_, i) => i !== at))}
      onToggleOrder={(at) =>
        onChange(index.columns.map((c, i) => (i === at ? { ...c, descending: !c.descending } : c)))
      }
    />
  );
}
