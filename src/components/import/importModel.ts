// 导入对话框的纯逻辑:勾选、文案。不碰后端,方便单测。
import type { CandidateStatus, ImportCandidate, ImportReport, PasswordStatus } from "../../lib/importTypes";
import { endpointSummary } from "../connection/driverMeta";

export function selectableIds(candidates: readonly ImportCandidate[]): string[] {
  return candidates.filter((c) => c.status.kind !== "unsupported").map((c) => c.id);
}

export function defaultSelection(candidates: readonly ImportCandidate[]): Set<string> {
  const checked = (c: ImportCandidate) => c.status.kind === "new" || c.status.kind === "fillPassword";
  return new Set(candidates.filter(checked).map((c) => c.id));
}

export function toggleSelection(
  selection: ReadonlySet<string>,
  candidates: readonly ImportCandidate[],
  id: string,
): Set<string> {
  const next = new Set(selection);
  if (next.has(id)) next.delete(id);
  else if (selectableIds(candidates).includes(id)) next.add(id);
  return next;
}

export function selectAll(candidates: readonly ImportCandidate[], checked: boolean): Set<string> {
  return new Set(checked ? selectableIds(candidates) : []);
}

export function selectionSummary(candidates: readonly ImportCandidate[], selection: ReadonlySet<string>): string {
  if (selection.size === 0) return "未选择连接";
  const withPassword = candidates.filter((c) => selection.has(c.id) && c.password === "included").length;
  return `已选 ${selection.size} 个，其中 ${withPassword} 个含密码`;
}

export function statusLabel(status: CandidateStatus): string {
  switch (status.kind) {
    case "new":
      return "新增";
    case "existing":
      return "已存在";
    case "fillPassword":
      return "补密码";
    case "unsupported":
      return "暂不支持";
  }
}

export function passwordLabel(password: PasswordStatus): string {
  switch (password) {
    case "included":
      return "含密码";
    case "absent":
      return "不含密码";
    case "undecryptable":
      return "无法解密";
  }
}

export function candidateAddress(candidate: ImportCandidate): string {
  const { endpoint } = candidate;
  return endpoint ? endpointSummary({ endpoint }) : "—";
}

export function reportHeadline(report: ImportReport): string {
  const parts = [
    report.imported.length > 0 ? `已导入 ${report.imported.length} 个连接` : null,
    report.passwordFilled.length > 0 ? `为 ${report.passwordFilled.length} 个已有连接补上了密码` : null,
  ].filter((part) => part !== null);
  const head = parts.length > 0 ? parts.join("，") : "没有导入任何连接";
  return report.skipped.length > 0 ? `${head}，跳过 ${report.skipped.length} 个` : head;
}
