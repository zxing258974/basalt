import { useEffect, useRef } from "react";
import type { ImportCandidate } from "../../lib/importTypes";
import { DriverBadge } from "../connection/DriverBadge";
import { candidateAddress, passwordLabel, selectAll, selectableIds, statusLabel, toggleSelection } from "./importModel";

interface ImportPreviewProps {
  candidates: readonly ImportCandidate[];
  selection: ReadonlySet<string>;
  onSelectionChange(next: Set<string>): void;
}

export function ImportPreview({ candidates, selection, onSelectionChange }: ImportPreviewProps) {
  if (candidates.length === 0) {
    return <p className="import-placeholder">这个来源里没有连接。</p>;
  }
  const selectable = selectableIds(candidates);
  const allChecked = selectable.length > 0 && selectable.every((id) => selection.has(id));
  const someChecked = selection.size > 0 && !allChecked;
  return (
    <div className="import-table-scroll">
      <table className="import-table">
        <thead>
          <tr>
            <th className="import-check-cell">
              <HeaderCheckbox
                checked={allChecked}
                indeterminate={someChecked}
                disabled={selectable.length === 0}
                onChange={(checked) => onSelectionChange(selectAll(candidates, checked))}
              />
            </th>
            <th>连接</th>
            <th className="import-type-col">类型</th>
            <th className="import-address-col">地址</th>
            <th className="import-password-col">密码</th>
            <th className="import-status-col">状态</th>
          </tr>
        </thead>
        <tbody>
          {candidates.map((candidate) => (
            <CandidateRow
              key={candidate.id}
              candidate={candidate}
              checked={selection.has(candidate.id)}
              onToggle={() => onSelectionChange(toggleSelection(selection, candidates, candidate.id))}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function HeaderCheckbox({
  checked,
  indeterminate,
  disabled,
  onChange,
}: {
  checked: boolean;
  indeterminate: boolean;
  disabled: boolean;
  onChange(checked: boolean): void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={ref}
      type="checkbox"
      aria-label="全选"
      checked={checked}
      disabled={disabled}
      onChange={(event) => onChange(event.target.checked)}
    />
  );
}

function CandidateRow({
  candidate,
  checked,
  onToggle,
}: {
  candidate: ImportCandidate;
  checked: boolean;
  onToggle(): void;
}) {
  const { status } = candidate;
  const unsupported = status.kind === "unsupported";
  return (
    <tr className={unsupported ? "is-unsupported" : undefined} onClick={unsupported ? undefined : onToggle}>
      <td className="import-check-cell">
        <input
          type="checkbox"
          aria-label={`选择「${candidate.name}」`}
          checked={checked}
          disabled={unsupported}
          onChange={onToggle}
          onClick={(event) => event.stopPropagation()}
        />
      </td>
      <td>
        <div className="import-name-stack">
          <span className="import-name">
            {candidate.name}
            {candidate.group && <span className="import-group">{candidate.group}</span>}
          </span>
          {status.kind === "existing" && <span className="import-note">与「{status.existingName}」重复</span>}
          {status.kind === "unsupported" && <span className="import-note">{status.reason}</span>}
          {candidate.warnings.map((warning) => (
            <span key={warning} className="import-note is-warning">
              {warning}
            </span>
          ))}
        </div>
      </td>
      <td>
        <span className="import-type">
          {candidate.driver && <DriverBadge driver={candidate.driver} />}
          {candidate.sourceType}
        </span>
      </td>
      <td className="import-address" title={candidateAddress(candidate)}>
        {candidateAddress(candidate)}
      </td>
      <td className={`import-password is-${candidate.password}`}>
        {unsupported ? "—" : passwordLabel(candidate.password)}
      </td>
      <td>
        <span className={`import-status is-${status.kind}`}>{statusLabel(status)}</span>
      </td>
    </tr>
  );
}
