// 导入 Navicat 查询的纯逻辑:勾选、状态文案、位置的写法。
import type { QueryImportCandidate, QueryImportReport, QueryImportStatus } from "../../lib/queryTypes";

export const importableQueries = (candidates: readonly QueryImportCandidate[]) =>
  candidates.filter((c) => c.status.kind === "new").map((c) => c.id);

/** 只有「新增」能导入,默认全勾。 */
export function defaultQuerySelection(candidates: readonly QueryImportCandidate[]): Set<string> {
  return new Set(importableQueries(candidates));
}

export function toggleQuery(selection: ReadonlySet<string>, candidates: readonly QueryImportCandidate[], id: string) {
  const next = new Set(selection);
  if (next.has(id)) next.delete(id);
  else if (importableQueries(candidates).includes(id)) next.add(id);
  return next;
}

export function queryStatusLabel(status: QueryImportStatus): string {
  switch (status.kind) {
    case "new":
      return "新增";
    case "exists":
      return "已导入";
    case "conflict":
      return "同名冲突";
    case "unmatched":
      return "无法导入";
  }
}

/** Navicat 里的位置:类型 · 连接 / 库 / schema。 */
export function sourcePath(candidate: QueryImportCandidate): string {
  const path = [candidate.sourceConnection, candidate.sourceDatabase, candidate.sourceSchema]
    .filter(Boolean)
    .join(" / ");
  return `${candidate.sourceType} · ${path}`;
}

/** Basalt 里的落脚处:连接 / 库 / schema;对不上时为 null。 */
export function targetPath(candidate: QueryImportCandidate): string | null {
  const target = candidate.target;
  if (!target) return null;
  return [target.connectionName, target.database, target.schema].filter(Boolean).join(" / ");
}

export function querySelectionSummary(candidates: readonly QueryImportCandidate[], selection: ReadonlySet<string>) {
  const blocked = candidates.length - importableQueries(candidates).length;
  const head = selection.size === 0 ? "未选择查询" : `已选 ${selection.size} 条`;
  return blocked > 0 ? `${head}，${blocked} 条不能导入` : head;
}

export function queryReportHeadline(report: QueryImportReport): string {
  const head = report.imported.length > 0 ? `已导入 ${report.imported.length} 条查询` : "没有导入任何查询";
  return report.skipped.length > 0 ? `${head}，跳过 ${report.skipped.length} 条` : head;
}
