// 导入向导的纯逻辑:映射的增改、必填列提示、按映射重排的预览、报告的写法。不碰后端和 DOM。
import type {
  ColumnMapping,
  ImportMode,
  ImportOptions,
  ImportReport,
  SourceFormat,
  SourcePreview,
  TextEncoding,
  UniqueKey,
} from "../../lib/transferTypes";
import type { ColumnInfo, DriverKind, TableRef } from "../../lib/types";

export interface ImportTarget {
  connectionId: string;
  driver: DriverKind;
  table: TableRef;
  /** 如 shop.customers。 */
  title: string;
}

export type WizardStep = "file" | "mode" | "mapping" | "options";

export const STEPS: { step: WizardStep; label: string }[] = [
  { step: "file", label: "选文件" },
  { step: "mode", label: "导入模式" },
  { step: "mapping", label: "映射列" },
  { step: "options", label: "选项" },
];

export const DEFAULT_IMPORT_OPTIONS: ImportOptions = {
  onError: "stop",
  batchSize: 500,
  mode: "append",
  keyColumns: [],
  binary: "hex",
};

export const IMPORT_MODES: { value: ImportMode; label: string; detail: string }[] = [
  { value: "append", label: "追加", detail: "把文件里的行全部插入表中。" },
  {
    value: "update",
    label: "更新",
    detail: "按匹配键找到表里已有的行，改成文件里的值；表里找不到的行跳过，结果里会写明有几行。",
  },
  {
    value: "upsert",
    label: "追加或更新",
    detail: "按主键或一个唯一索引匹配：表里已有的更新，没有的插入。",
  },
  {
    value: "delete",
    label: "删除",
    detail: "按匹配键删掉表里与文件对应的行；只用到匹配键这几列，其余列不看。",
  },
  { value: "replace", label: "复制", detail: "先清空表里现有的全部行，再把文件里的行全部插入。" },
];

export function modeLabel(mode: ImportMode): string {
  return IMPORT_MODES.find((m) => m.value === mode)!.label;
}

/** 更新、追加或更新、删除要用匹配键找到表里对应的行。 */
export function usesKeys(mode: ImportMode): boolean {
  return mode === "update" || mode === "upsert" || mode === "delete";
}

/** 会删掉表里已有数据的模式:开始前要二次确认。 */
export function isDestructive(mode: ImportMode): boolean {
  return mode === "replace" || mode === "delete";
}

const mappedColumns = (mapping: ColumnMapping[]) =>
  new Set(mapping.filter((m) => m.source !== null).map((m) => m.column));

const sameColumns = (key: UniqueKey, columns: string[]) =>
  key.columns.length === columns.length && key.columns.every((c) => columns.includes(c));

/** 进映射这一步时的默认匹配键:主键;追加或更新取第一个所有列都映射了的键。 */
export function defaultKeys(mode: ImportMode, uniqueKeys: UniqueKey[], mapping: ColumnMapping[]): string[] {
  if (!usesKeys(mode)) return [];
  const mapped = mappedColumns(mapping);
  const usable = uniqueKeys.filter((key) => key.columns.every((c) => mapped.has(c)));
  if (mode === "upsert") return usable[0]?.columns ?? [];
  // 只映射了复合主键的一部分时不默认:按半个主键删、改,范围会比想的大得多。
  const primary = usable.find((key) => key.name === null);
  return primary?.columns ?? [];
}

/** 匹配键选得对不对;对的话返回 null。 */
export function keyProblem(
  mode: ImportMode,
  keys: string[],
  uniqueKeys: UniqueKey[],
  mapping: ColumnMapping[],
): string | null {
  if (!usesKeys(mode)) return null;
  const label = modeLabel(mode);
  if (keys.length === 0) return `「${label}」要勾选匹配键：按哪几列找到表里对应的行。`;
  const mapped = mappedColumns(mapping);
  const unmapped = keys.filter((k) => !mapped.has(k));
  if (unmapped.length > 0) return `匹配键${unmapped.map((k) => `「${k}」`).join("")}没有选来源列。`;
  if (mode === "upsert" && !uniqueKeys.some((key) => sameColumns(key, keys))) {
    return "「追加或更新」只能按主键或一个唯一索引匹配，勾选的列要和其中一个完全一样。";
  }
  if (mode === "update" && [...mapped].every((c) => keys.includes(c))) {
    return "「更新」除了匹配键，至少还要给一列要改的列选上来源。";
  }
  return null;
}

/** 如「主键（id）」「唯一索引 uk_code（code）」。 */
export function describeKey(key: UniqueKey): string {
  const columns = key.columns.join("、");
  return key.name === null ? `主键（${columns}）` : `唯一索引 ${key.name}（${columns}）`;
}

/** 开始前的二次确认;不需要确认的模式返回 null。 */
export function confirmation(
  mode: ImportMode,
  tableTitle: string,
  driver: DriverKind,
): { title: string; text: string; action: string } | null {
  if (mode === "replace") {
    return {
      title: "清空表再导入？",
      text: `会先删掉「${tableTitle}」里现有的全部行，再导入文件里的数据。${commitNote(driver, mode)}`,
      action: "清空并导入",
    };
  }
  if (mode === "delete") {
    return {
      title: "按文件删除表里的行？",
      text: `会按匹配键删掉「${tableTitle}」里与文件对应的行，文件本身不会写进表里。${commitNote(driver, mode)}`,
      action: "删除",
    };
  }
  return null;
}

/** MySQL 的「追加或更新」遇到任何一个唯一键冲突都算「已有」,不只看选中的键。 */
export function mysqlUpsertNote(driver: DriverKind, mode: ImportMode, uniqueKeys: UniqueKey[]): string | null {
  const mysql = driver === "mysql" || driver === "mariadb";
  if (!mysql || mode !== "upsert" || uniqueKeys.length < 2) return null;
  return "这张表有不止一个唯一键。MySQL 判断「已有」时会看所有唯一键：文件里的一行只要和任何一个撞上，就会更新撞上的那一行，而不是新增。";
}

/** 结果页的「生效 N 行」按模式换个说法。 */
export function affectedLabel(mode: ImportMode): string {
  switch (mode) {
    case "update":
      return "更新";
    case "upsert":
      return "处理";
    case "delete":
      return "删除";
    default:
      return "插入";
  }
}

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

/** 有不止一个工作表的 Excel:要用户亲手选一个,不默认用第一个。 */
export function needsSheetChoice(source: SourcePreview): boolean {
  return source.format.kind === "excel" && source.sheets.length > 1;
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

/** MySQL 每批单独提交(「复制」除外),选项页和确认时要说清楚。 */
export function commitNote(driver: DriverKind, mode: ImportMode = "append"): string {
  const mysql = driver === "mysql" || driver === "mariadb";
  if (mysql && mode === "replace") {
    return "清空和导入在同一个事务里：中途出错或取消时一起撤回，原来的数据不会丢（MyISAM 这类不支持事务的引擎除外）。";
  }
  return mysql
    ? "MySQL 每批单独提交：中途出错或取消时，已经提交的批次会留在表里，结果里会写明到哪一行。"
    : "整个导入在一个事务里：中途出错或取消时全部回滚，表保持原样。";
}
