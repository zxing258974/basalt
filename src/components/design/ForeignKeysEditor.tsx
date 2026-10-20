import { ArrowRight, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTables } from "../../hooks/useCatalog";
import { useTableDesign } from "../../hooks/useTableDesign";
import type { FkAction, ForeignKeyDef, TableDesign } from "../../lib/designTypes";
import { Button } from "../ui/Button";
import { ColumnChips } from "./ColumnChips";
import { Combo } from "./Combo";
import * as model from "./designModel";

interface ForeignKeysEditorProps {
  connectionId: string;
  draft: TableDesign;
  onChange(next: TableDesign): void;
}

const ACTIONS: { value: FkAction; label: string }[] = [
  { value: "noAction", label: "NO ACTION" },
  { value: "restrict", label: "RESTRICT" },
  { value: "cascade", label: "CASCADE" },
  { value: "setNull", label: "SET NULL" },
  { value: "setDefault", label: "SET DEFAULT" },
];

/** 外键页:本表字段 → 引用表的字段,加上删除、更新时的动作。 */
export function ForeignKeysEditor({ connectionId, draft, onChange }: ForeignKeysEditorProps) {
  const { database, schema } = draft.table;
  const tables = useTables(connectionId, database, schema, true).data ?? [];
  const tableNames = tables.filter((t) => t.kind === "table").map((t) => t.name);
  return (
    <div className="designer-pane">
      <div className="designer-toolbar" role="toolbar" aria-label="外键操作">
        <Button size="sm" variant="ghost" icon={<Plus />} onClick={() => onChange(model.addForeignKey(draft))}>
          新增外键
        </Button>
      </div>
      <div className="design-cards">
        {draft.foreignKeys.length === 0 && <p className="designer-empty">这张表还没有外键。</p>}
        {draft.foreignKeys.map((fk, index) => (
          <ForeignKeyCard
            key={fk.id}
            connectionId={connectionId}
            draft={draft}
            fk={fk}
            label={fk.name || `第 ${index + 1} 个外键`}
            tableNames={tableNames}
            onChange={(patch) => onChange(model.updateForeignKey(draft, fk.id, patch))}
            onRemove={() => onChange(model.removeForeignKey(draft, fk.id))}
          />
        ))}
      </div>
    </div>
  );
}

interface ForeignKeyCardProps {
  connectionId: string;
  draft: TableDesign;
  fk: ForeignKeyDef;
  label: string;
  tableNames: string[];
  onChange(patch: Partial<ForeignKeyDef>): void;
  onRemove(): void;
}

function ForeignKeyCard({ connectionId, draft, fk, label, tableNames, onChange, onRemove }: ForeignKeyCardProps) {
  const nameOf = (id: string) => draft.columns.find((c) => c.id === id)?.name || "（未命名字段）";
  const used = new Set(fk.columns);
  return (
    <article className="design-card" aria-label={`外键 ${label}`}>
      <div className="design-card-head">
        <input
          className="design-card-name mono"
          aria-label="外键名"
          value={fk.name}
          placeholder="外键名（留空由数据库命名）"
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          onChange={(event) => onChange({ name: event.target.value })}
        />
        <span className="design-card-spacer" />
        <Button size="sm" variant="ghost" icon={<Trash2 />} aria-label={`删除外键 ${label}`} onClick={onRemove} />
      </div>
      <div className="fk-mapping">
        <ColumnChips
          label={`${label} 的字段`}
          chips={fk.columns.map((id) => ({ value: id, label: nameOf(id) }))}
          options={draft.columns.filter((c) => !used.has(c.id)).map((c) => ({ value: c.id, label: nameOf(c.id) }))}
          emptyHint="本表字段"
          onAdd={(id) => onChange({ columns: [...fk.columns, id] })}
          onRemove={(at) => onChange({ columns: fk.columns.filter((_, i) => i !== at) })}
        />
        <ArrowRight className="fk-arrow" aria-hidden />
        <div className="fk-target">
          {fk.refSchema && <span className="fk-schema mono">{fk.refSchema}.</span>}
          <Combo
            className="fk-table"
            label={`${label}：引用的表`}
            value={fk.refTable}
            options={tableNames}
            placeholder="引用的表"
            onChange={(refTable) => onChange({ refTable, refColumns: [] })}
          />
          <ReferencedColumns
            connectionId={connectionId}
            draft={draft}
            fk={fk}
            label={label}
            known={tableNames.includes(fk.refTable)}
            onChange={(refColumns) => onChange({ refColumns })}
          />
        </div>
      </div>
      <div className="fk-actions">
        <ActionSelect label={`${label}：删除时`} value={fk.onDelete} onChange={(onDelete) => onChange({ onDelete })} />
        <ActionSelect label={`${label}：更新时`} value={fk.onUpdate} onChange={(onUpdate) => onChange({ onUpdate })} />
      </div>
    </article>
  );
}

/** 引用表的字段:读得到那张表就从它的字段里选,读不到(别的库、还没建)就手填,逗号分隔。 */
function ReferencedColumns({
  connectionId,
  draft,
  fk,
  label,
  known,
  onChange,
}: {
  connectionId: string;
  draft: TableDesign;
  fk: ForeignKeyDef;
  label: string;
  known: boolean;
  onChange(columns: string[]): void;
}) {
  const ref = { database: draft.table.database, schema: fk.refSchema ?? draft.table.schema, table: fk.refTable };
  const referenced = useTableDesign(connectionId, ref, known && !fk.refSchema).data;
  if (!referenced) {
    return <ManualColumns label={label} columns={fk.refColumns} onChange={onChange} />;
  }
  const used = new Set(fk.refColumns);
  return (
    <ColumnChips
      label={`${label} 引用的字段`}
      chips={fk.refColumns.map((name) => ({ value: name, label: name }))}
      options={referenced.columns.filter((c) => !used.has(c.name)).map((c) => ({ value: c.name, label: c.name }))}
      emptyHint="引用的字段"
      onAdd={(name) => onChange([...fk.refColumns, name])}
      onRemove={(at) => onChange(fk.refColumns.filter((_, i) => i !== at))}
    />
  );
}

const parse = (text: string) =>
  text
    .split(",")
    .map((c) => c.trim())
    .filter(Boolean);

/** 手填的字段列表:输入框里保留原文(含没打完的逗号),只把解析结果交出去。 */
function ManualColumns({
  label,
  columns,
  onChange,
}: {
  label: string;
  columns: string[];
  onChange(c: string[]): void;
}) {
  const [text, setText] = useState(columns.join(", "));
  if (parse(text).join(",") !== columns.join(",")) setText(columns.join(", "));
  return (
    <input
      className="design-input mono"
      aria-label={`${label}：引用的字段`}
      value={text}
      placeholder="引用的字段，逗号分隔"
      spellCheck={false}
      autoCorrect="off"
      autoCapitalize="off"
      onChange={(event) => {
        setText(event.target.value);
        onChange(parse(event.target.value));
      }}
    />
  );
}

function ActionSelect({ label, value, onChange }: { label: string; value: FkAction; onChange(v: FkAction): void }) {
  return (
    <label className="fk-action">
      <span>{label.split("：")[1]}</span>
      <select
        className="design-select"
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value as FkAction)}
      >
        {ACTIONS.map((action) => (
          <option key={action.value} value={action.value}>
            {action.label}
          </option>
        ))}
      </select>
    </label>
  );
}
