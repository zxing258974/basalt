import { ArrowDown, ArrowUp, KeyRound, ListPlus, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import type { ColumnDef, TableDesign } from "../../lib/designTypes";
import type { DriverKind } from "../../lib/types";
import { Button } from "../ui/Button";
import { Combo } from "./Combo";
import * as model from "./designModel";

interface ColumnsEditorProps {
  driver: DriverKind;
  original: TableDesign | null;
  draft: TableDesign;
  onChange(next: TableDesign): void;
  selectedId: string | null;
  onSelect(id: string | null): void;
}

const STATUS_LABEL: Record<model.ColumnStatus, string | undefined> = {
  added: "新字段",
  edited: "已修改",
  unchanged: undefined,
};

/** 字段页:可编辑的字段表格,上方工具条增删、挪动、设主键。 */
export function ColumnsEditor({ driver, original, draft, onChange, selectedId, onSelect }: ColumnsEditorProps) {
  const selected = draft.columns.find((c) => c.id === selectedId) ?? null;
  const selectedIndex = selected ? draft.columns.indexOf(selected) : -1;
  const hasComments = driver !== "sqlite";
  const types = model.typeSuggestions(driver);
  // 新加的那一行挂载时就把焦点放进字段名,紧接着打字不会丢字。
  const [focusId, setFocusId] = useState<string | null>(null);

  const add = (at?: number) => {
    const [next, id] = model.addColumn(draft, driver, at);
    onChange(next);
    onSelect(id);
    setFocusId(id);
  };
  const move = (delta: -1 | 1) => {
    if (selected && model.canMove(driver, original, draft, selected.id, delta)) {
      onChange(model.moveColumn(draft, selected.id, delta));
    }
  };
  const update = (id: string, patch: Partial<ColumnDef>) => onChange(model.updateColumn(draft, id, patch));
  const remove = () => {
    if (!selected) return;
    onChange(model.removeColumn(draft, selected.id));
    const next = draft.columns[selectedIndex + 1] ?? draft.columns[selectedIndex - 1];
    onSelect(next && next.id !== selected.id ? next.id : null);
  };

  return (
    <div className="designer-pane">
      <div className="designer-toolbar" role="toolbar" aria-label="字段操作">
        <Button size="sm" variant="ghost" icon={<Plus />} onClick={() => add()}>
          新增字段
        </Button>
        <Button size="sm" variant="ghost" icon={<ListPlus />} disabled={!selected} onClick={() => add(selectedIndex)}>
          插入
        </Button>
        <Button size="sm" variant="ghost" icon={<Trash2 />} disabled={!selected} onClick={remove}>
          删除
        </Button>
        <span className="designer-toolbar-divider" />
        <Button
          size="sm"
          variant="ghost"
          icon={<ArrowUp />}
          disabled={!selected || !model.canMove(driver, original, draft, selected.id, -1)}
          onClick={() => move(-1)}
        >
          上移
        </Button>
        <Button
          size="sm"
          variant="ghost"
          icon={<ArrowDown />}
          disabled={!selected || !model.canMove(driver, original, draft, selected.id, 1)}
          onClick={() => move(1)}
        >
          下移
        </Button>
        <span className="designer-toolbar-divider" />
        <Button
          size="sm"
          variant="ghost"
          icon={<KeyRound />}
          disabled={!selected}
          aria-pressed={selected ? draft.primaryKey.includes(selected.id) : false}
          onClick={() => selected && onChange(model.togglePrimaryKey(draft, selected.id))}
        >
          主键
        </Button>
        {driver === "postgres" && original && (
          <span className="designer-toolbar-hint">PostgreSQL 不能调整已有字段的顺序，新字段加在最后。</span>
        )}
      </div>

      <div className="column-grid-scroll">
        <table className={hasComments ? "column-grid" : "column-grid no-comment"} aria-label="字段">
          <thead>
            <tr>
              <th scope="col" className="col-mark">
                <span className="visually-hidden">序号</span>
              </th>
              <th scope="col">字段名</th>
              <th scope="col">类型</th>
              <th scope="col" className="col-check">
                非空
              </th>
              <th scope="col" className="col-check">
                主键
              </th>
              <th scope="col">默认值</th>
              <th scope="col" className="col-check">
                自增
              </th>
              {hasComments && <th scope="col">注释</th>}
            </tr>
          </thead>
          <tbody>
            {draft.columns.map((column, index) => (
              <ColumnRow
                key={column.id}
                index={index}
                column={column}
                status={model.columnStatus(original, column)}
                keyPosition={draft.primaryKey.indexOf(column.id)}
                selected={column.id === selectedId}
                autoFocus={column.id === focusId}
                hasComments={hasComments}
                types={types}
                onSelect={() => onSelect(column.id)}
                onUpdate={(patch) => update(column.id, patch)}
                onToggleKey={() => onChange(model.togglePrimaryKey(draft, column.id))}
              />
            ))}
          </tbody>
        </table>
        {draft.columns.length === 0 && <p className="designer-empty">还没有字段，点「新增字段」加一个。</p>}
      </div>

      {selected && <ColumnExtras column={selected} />}
    </div>
  );
}

interface ColumnRowProps {
  index: number;
  column: ColumnDef;
  status: model.ColumnStatus;
  /** 在主键里的位置,-1 表示不在主键里。 */
  keyPosition: number;
  selected: boolean;
  autoFocus: boolean;
  hasComments: boolean;
  types: string[];
  onSelect(): void;
  onUpdate(patch: Partial<ColumnDef>): void;
  onToggleKey(): void;
}

function ColumnRow(props: ColumnRowProps) {
  const { index, column, status, keyPosition, selected, hasComments } = props;
  const name = column.name || `第 ${index + 1} 个字段`;
  const generated = column.generated !== null;
  const classes = ["column-row", `is-${status}`, selected && "is-selected"].filter(Boolean).join(" ");
  return (
    <tr className={classes} aria-selected={selected} onFocusCapture={props.onSelect} onMouseDown={props.onSelect}>
      <td className="col-mark">
        <span className="column-number mono">{index + 1}</span>
        {keyPosition >= 0 && (
          <span className="column-key" title={`主键第 ${keyPosition + 1} 列`}>
            <KeyRound aria-hidden />
          </span>
        )}
        {STATUS_LABEL[status] && <span className="visually-hidden">（{STATUS_LABEL[status]}）</span>}
      </td>
      <td>
        <input
          className="cell-input mono"
          autoFocus={props.autoFocus}
          aria-label={`${name}：字段名`}
          value={column.name}
          placeholder="字段名"
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          onChange={(event) => props.onUpdate({ name: event.target.value })}
        />
      </td>
      <td>
        <Combo
          className="cell-combo"
          label={`${name}：类型`}
          value={column.dataType}
          options={props.types}
          placeholder="类型"
          onChange={(dataType) => props.onUpdate({ dataType })}
        />
      </td>
      <td className="col-check">
        <input
          type="checkbox"
          aria-label={`${name}：非空`}
          checked={!column.nullable}
          onChange={(event) => props.onUpdate({ nullable: !event.target.checked })}
        />
      </td>
      <td className="col-check">
        <input type="checkbox" aria-label={`${name}：主键`} checked={keyPosition >= 0} onChange={props.onToggleKey} />
      </td>
      <td>
        <input
          className="cell-input mono"
          aria-label={`${name}：默认值`}
          value={column.default ?? ""}
          placeholder={generated ? "生成列" : column.autoIncrement ? "自增" : "无"}
          disabled={generated || column.autoIncrement}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          onChange={(event) => props.onUpdate({ default: event.target.value === "" ? null : event.target.value })}
        />
      </td>
      <td className="col-check">
        <input
          type="checkbox"
          aria-label={`${name}：自增`}
          checked={column.autoIncrement}
          disabled={generated}
          onChange={(event) => props.onUpdate({ autoIncrement: event.target.checked, default: null })}
        />
      </td>
      {hasComments && (
        <td>
          <input
            className="cell-input"
            aria-label={`${name}：注释`}
            value={column.comment ?? ""}
            placeholder="—"
            onChange={(event) => props.onUpdate({ comment: event.target.value === "" ? null : event.target.value })}
          />
        </td>
      )}
    </tr>
  );
}

/** 选中字段上设计器不改、但会原样保留的属性。 */
function ColumnExtras({ column }: { column: ColumnDef }) {
  const extras = [
    column.collation && ["排序规则", column.collation],
    column.onUpdate && ["ON UPDATE", column.onUpdate],
    column.generated && [
      column.generated.stored ? "生成列（存储）" : "生成列（虚拟）",
      column.generated.expression || "（表达式读不出来）",
    ],
  ].filter((item): item is [string, string] => Boolean(item));
  if (extras.length === 0) return null;
  return (
    <dl className="column-extras" aria-label={`${column.name} 的其它属性`}>
      {extras.map(([term, detail]) => (
        <div key={term} className="column-extra">
          <dt>{term}</dt>
          <dd className="mono selectable">{detail}</dd>
        </div>
      ))}
      <p className="column-extras-note">这些属性在这里不能改，保存时原样保留。</p>
    </dl>
  );
}
