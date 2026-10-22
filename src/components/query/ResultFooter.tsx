import { Lock } from "lucide-react";
import { formatCount } from "../../lib/format";
import { shortcut } from "../../lib/platform";
import type { TableRef } from "../../lib/types";
import { EditActions, type EditActionsProps } from "../data/EditActions";
import { Button } from "../ui/Button";
import { sameTable, tableLabel } from "./resultEditing";

interface ResultFooterProps extends EditActionsProps {
  loaded: number;
  total: number;
  /** 结果太大,后端只存了前 total 行。 */
  truncated: boolean;
  loading: boolean;
  error: string | null;
  onRetry(): void;
  /** 不能改的原因;能改时为 null。 */
  readOnlyReason: string | null;
  /** 最近一次提交的结果。 */
  message: string | null;
  /** 连接了几张表、不止一张能改时可选的表;否则为空。 */
  editTables: readonly TableRef[];
  editTable: TableRef | null;
  onEditTableChange(table: TableRef): void;
}

/** 结果表格底下:取到了多少行、能不能改,能改时放增删行、撤销、提交。 */
export function ResultFooter(props: ResultFooterProps) {
  const { loaded, total, pending } = props;
  const count =
    loaded >= total ? `${formatCount(total)} 行` : `已显示 ${formatCount(loaded)} / 共 ${formatCount(total)} 行`;
  return (
    <footer className="result-footer">
      <span className="result-footer-count mono">
        {count}
        {props.truncated && "（结果太大，只保留了这些）"}
      </span>
      {props.loading && <span className="result-footer-loading">正在读取…</span>}
      {props.error && (
        <span className="result-footer-error" role="alert">
          读取后面的行失败：{props.error}
          <Button variant="ghost" size="sm" onClick={props.onRetry}>
            重试
          </Button>
        </span>
      )}
      <span className="result-footer-status">
        {props.readOnlyReason !== null ? (
          <span className="result-footer-readonly" title={props.readOnlyReason}>
            <Lock aria-hidden />
            {props.readOnlyReason}
          </span>
        ) : pending > 0 ? (
          <span className="data-footer-pending">
            {pending} 行有未提交的修改 · <kbd>{shortcut("S")}</kbd> 预览并提交
          </span>
        ) : props.message ? (
          <span className="data-footer-message" role="status">
            {props.message}
          </span>
        ) : null}
      </span>
      {props.editTables.length > 1 && (
        <EditTablePicker
          tables={props.editTables}
          value={props.editTable}
          // 改了一半换表的话,手上的修改对不上新表的列。
          disabled={pending > 0}
          onChange={props.onEditTableChange}
        />
      )}
      {props.readOnlyReason === null && (
        <div className="data-actions">
          <EditActions {...props} />
        </div>
      )}
    </footer>
  );
}

function EditTablePicker(props: {
  tables: readonly TableRef[];
  value: TableRef | null;
  disabled: boolean;
  onChange(table: TableRef): void;
}) {
  const key = (table: TableRef) => [table.database, table.schema ?? "", table.table].join("\u0000");
  const value = props.value;
  const index = value ? props.tables.findIndex((t) => sameTable(t, value)) : 0;
  // 两个库里同名的表(MySQL 跨库连接):带上库名才分得开。
  const labels = props.tables.map(tableLabel);
  const label = (table: TableRef, i: number) =>
    labels.indexOf(labels[i]!) === labels.lastIndexOf(labels[i]!) ? labels[i]! : `${table.database}.${labels[i]!}`;
  return (
    <label className="result-footer-table" title={props.disabled ? "先提交或撤销修改再换" : undefined}>
      <span>改</span>
      <select
        className="field-select"
        aria-label="改哪张表"
        value={Math.max(index, 0)}
        disabled={props.disabled}
        onChange={(event) => props.onChange(props.tables[Number(event.target.value)]!)}
      >
        {props.tables.map((table, i) => (
          <option key={key(table)} value={i}>
            {label(table, i)}
          </option>
        ))}
      </select>
    </label>
  );
}
