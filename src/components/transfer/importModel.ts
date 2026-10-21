// 导入向导的纯逻辑:映射的增改、必填列提示、按映射重排的预览、报告的写法。不碰后端和 DOM。
import type {
  ColumnMapping,
  ImportOptions,
  ImportReport,
  SourceFormat,
  SourcePreview,
  TextEncoding,
} from "../../lib/transferTypes";
import type { ColumnInfo, DriverKind, TableRef } from "../../lib/types";

export interface ImportTarget {
  connectionId: string;
  driver: DriverKind;
  table: TableRef;
  /** 如 shop.customers。 */
  title: string;
}

export type WizardStep = "file" | "mapping" | "options";

export const STEPS: { step: WizardStep; label: string }[] = [
  { step: "file", label: "选文件" },
  { step: "mapping", label: "映射列" },
  { step: "options", label: "选项" },
];

export const DEFAULT_IMPORT_OPTIONS: ImportOptions = {
  onError: "stop",
  batchSize: 500,
  clearTable: false,
  binary: "hex",
};

export const MAX_BATCH_SIZE = 10_000;

/** 编码、分隔符下拉框里的「自动」。 */
export const AUTO = "auto";

export const ENCODING_CHOICES: { value: TextEncoding | typeof AUTO; label: string }[] = [
  { value: AUTO, label: "自动识别" },
  { value: "utf8", label: "UTF-8" },
  { value: "gbk", label: "GBK" },
];

export const DELIMITER_CHOICES: { value: string; label: string }[] = [
  { value: AUTO, label: "自动识别" },
  { value: ",", label: "逗号" },
  { value: "\t", label: "制表符" },
  { value: ";", label: "分号" },
  { value: "|", label: "竖线" },
];

/** 把某一列的来源改成 `source`(null 是不导入)。 */
export function setSource(mapping: ColumnMapping[], column: string, source: number | null): ColumnMapping[] {
  return mapping.map((item) => (item.column === column ? { ...item, source } : item));
}

export function mappedCount(mapping: ColumnMapping[]): number {
  return mapping.filter((item) => item.source !== null).length;
}

/** 不能为空、又没有默认值,却没有映射的列:每一行都会失败。 */
export function missingRequired(columns: ColumnInfo[], mapping: ColumnMapping[]): ColumnInfo[] {
  const mapped = new Set(mapping.filter((m) => m.source !== null).map((m) => m.column));
  return columns.filter((c) => !c.nullable && !c.hasDefault && !c.isGenerated && !mapped.has(c.name));
}

/** 预览表:只列已映射的目标列,值按映射从来源里取。 */
export function mappedPreview(preview: SourcePreview, mapping: ColumnMapping[]) {
  const used = mapping.filter((m) => m.source !== null);
  return {
    headers: used.map((m) => m.column),
    sources: used.map((m) => preview.headers[m.source!] ?? `第 ${m.source! + 1} 列`),
    rows: preview.rows.map((row) => used.map((m) => row[m.source!] ?? null)),
  };
}

/** 文件格式说明:CSV 的编码与分隔符、Excel 的工作表。 */
export function describeFormat(format: SourceFormat): string {
  if (format.kind === "excel") return `Excel · 工作表「${format.sheet ?? "第一个"}」`;
  const encoding = format.encoding === "gbk" ? "GBK" : "UTF-8";
  const delimiter = DELIMITER_CHOICES.find((d) => d.value === format.delimiter)?.label ?? format.delimiter ?? "逗号";
  return `CSV · ${encoding} · ${delimiter}分隔`;
}

export type ReportTone = "success" | "warning" | "error";

export function reportTone(report: ImportReport): ReportTone {
  if (report.status === "completed") return report.failed > 0 ? "warning" : "success";
  return report.status === "cancelled" ? "warning" : "error";
}

export function reportTitle(report: ImportReport): string {
  switch (report.status) {
    case "completed":
      return report.failed > 0 ? "导入完成，有些行失败了" : "导入完成";
    case "stopped":
      return "遇到错误，已停止";
    case "cancelled":
      return "已取消";
    case "failed":
      return "导入中途出错";
  }
}

export function failuresFileName(table: TableRef): string {
  return `${table.table}-导入失败.csv`;
}

/** MySQL 每批单独提交,选项页和确认清空时要说清楚。 */
export function commitNote(driver: DriverKind): string {
  return driver === "mysql" || driver === "mariadb"
    ? "MySQL 每批单独提交：中途出错或取消时，已经提交的批次会留在表里，结果里会写明到哪一行。"
    : "整个导入在一个事务里：中途出错或取消时全部回滚，表保持原样。";
}
