// 导出对话框的纯逻辑:格式选项、导出范围、文件名、数字与进度的写法。不碰后端和 DOM。
import type { FileFilter } from "../../lib/api";
import type { QueryScope, ResultSet } from "../../lib/queryTypes";
import type {
  BinaryEncoding,
  CsvOptions,
  ExportFormat,
  ExportFormatKind,
  ExportSource,
  TransferPhase,
} from "../../lib/transferTypes";
import type { DriverKind, SortSpec, TableRef } from "../../lib/types";

/** 从哪里导出:表(可以带着表数据页当前的筛选),或查询的一个结果集。 */
export type ExportScope =
  | { kind: "table"; table: TableRef; filter: string | null; sort: SortSpec | null }
  | { kind: "query"; editorId: string; scope: QueryScope; sql: string; set: ResultSet };

export interface ExportTarget {
  connectionId: string;
  driver: DriverKind;
  /** 对话框副标题里的名字,如 shop.customers、查询 1 的结果 1。 */
  title: string;
  /** 默认文件名(不含扩展名),也是 SQL INSERT 的默认表名。 */
  baseName: string;
  scope: ExportScope;
}

/** 表:整张表,还是表数据页当前筛选条件下的结果。 */
export type RangeChoice = "all" | "filtered";

export const DEFAULT_CSV: CsvOptions = { delimiter: ",", quote: '"', header: true, encoding: "utf8" };
export const DEFAULT_ROWS_PER_STATEMENT = 100;

export const FORMATS: { kind: ExportFormatKind; label: string; hint: string }[] = [
  { kind: "csv", label: "CSV", hint: "文本表格" },
  { kind: "xlsx", label: "Excel", hint: ".xlsx 工作簿" },
  { kind: "json", label: "JSON", hint: "数组或逐行" },
  { kind: "sql", label: "SQL", hint: "INSERT 语句" },
];

export const DELIMITERS = [
  { value: ",", label: "逗号" },
  { value: "\t", label: "制表符" },
  { value: ";", label: "分号" },
  { value: "|", label: "竖线" },
];

export const ENCODINGS = [
  { value: "utf8", label: "UTF-8" },
  { value: "utf8Bom", label: "UTF-8 带 BOM（Windows 上的 Excel 直接打开不乱码）" },
  { value: "gbk", label: "GBK" },
] as const;

export interface FormatChoices {
  kind: ExportFormatKind;
  csv: CsvOptions;
  jsonLines: boolean;
  sqlTable: string;
  rowsPerStatement: number;
}

export function buildFormat(choices: FormatChoices): ExportFormat {
  switch (choices.kind) {
    case "csv":
      return { kind: "csv", ...choices.csv };
    case "xlsx":
      return { kind: "xlsx" };
    case "json":
      return { kind: "json", lines: choices.jsonLines };
    case "sql":
      return { kind: "sql", table: choices.sqlTable.trim(), rowsPerStatement: Math.max(1, choices.rowsPerStatement) };
  }
}

export function extensionOf(format: ExportFormat): string {
  switch (format.kind) {
    case "csv":
      return format.delimiter === "\t" ? "tsv" : "csv";
    case "xlsx":
      return "xlsx";
    case "json":
      return format.lines ? "jsonl" : "json";
    case "sql":
      return "sql";
  }
}

export function fileFilter(format: ExportFormat): FileFilter {
  const names: Record<ExportFormatKind, string> = { csv: "CSV", xlsx: "Excel 工作簿", json: "JSON", sql: "SQL 脚本" };
  return { name: names[format.kind], extensions: [extensionOf(format)] };
}

/** 文件名里不能有的字符换成下划线。 */
export function safeFileName(name: string): string {
  return name.replace(/[/\\:*?"<>|]/g, "_").trim() || "export";
}

/** 表数据页带着筛选时默认导出筛选后的结果。 */
export function defaultRange(target: ExportTarget): RangeChoice {
  return target.scope.kind === "table" && target.scope.filter ? "filtered" : "all";
}

/** 交给后端的数据来源。查询结果没超出显示上限就直接写显示的行,不再执行一遍。 */
export function buildSource(target: ExportTarget, range: RangeChoice): ExportSource {
  const { scope } = target;
  if (scope.kind === "table") {
    const filtered = range === "filtered" && scope.filter !== null;
    return {
      kind: "table",
      table: scope.table,
      filter: filtered ? scope.filter : null,
      sort: filtered ? scope.sort : null,
    };
  }
  if (scope.set.truncated) return { kind: "query", editorId: scope.editorId, scope: scope.scope, sql: scope.sql };
  return { kind: "rows", columns: scope.set.columns, rows: scope.set.rows };
}

/** 查询结果的导出范围说明。 */
export function querySummary(set: ResultSet): string {
  return set.truncated
    ? "结果超出了显示上限：导出时会在这个查询标签页的连接上重新执行这条语句，读出全部结果。"
    : `导出显示的全部 ${formatCount(set.rows.length)} 行。`;
}

/** SQL INSERT 默认的表名:语句里只有一张表时用它,否则 query_result。 */
const IDENT = '(?:`[^`]+`|"[^"]+"|\\[[^\\]]+\\]|[\\w$]+)';
const TABLE_REF = new RegExp(`\\b(?:from|join)\\s+((?:${IDENT}\\.)*${IDENT})`, "gi");
const LAST_PART = new RegExp(`${IDENT}$`);

export function guessTableName(sql: string): string {
  const tables = [...sql.matchAll(TABLE_REF)].map((m) => m[1]!);
  if (tables.length !== 1) return "query_result";
  const last = LAST_PART.exec(tables[0]!)?.[0] ?? "";
  return last.replace(/^[`"[]|[`"\]]$/g, "") || "query_result";
}

export function formatCount(n: number): string {
  return n.toLocaleString("zh-CN");
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function formatElapsed(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function phaseLabel(phase: TransferPhase | null): string {
  switch (phase) {
    case "exporting":
      return "正在导出";
    case "checking":
      return "正在检查文件";
    case "importing":
      return "正在导入";
    case "finishing":
      return "正在收尾";
    default:
      return "正在准备";
  }
}

export function fileName(path: string): string {
  return path.split("/").at(-1) ?? path;
}

export const BINARY_CHOICES: { value: BinaryEncoding; label: string }[] = [
  { value: "hex", label: "十六进制（0x 开头）" },
  { value: "base64", label: "Base64" },
];
