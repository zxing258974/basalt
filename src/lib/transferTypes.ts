// 导入导出:与 crates/basalt-core/src/transfer 和 src-tauri/src/service/transfer.rs 的序列化结构对应。
import type { QueryScope, ResultColumn, ResultSort } from "./queryTypes";
import type { ColumnInfo, SortSpec, TableRef } from "./types";

export type BinaryEncoding = "hex" | "base64";
export type TextEncoding = "utf8" | "utf8Bom" | "gbk";

export interface CsvOptions {
  delimiter: string;
  quote: string;
  /** 第一行写列名。 */
  header: boolean;
  encoding: TextEncoding;
}

export type ExportFormat =
  | ({ kind: "csv" } & CsvOptions)
  | { kind: "xlsx" }
  /** lines:每行一个对象(JSON Lines),否则是一个数组。 */
  | { kind: "json"; lines: boolean }
  | { kind: "sql"; table: string; rowsPerStatement: number };

export type ExportFormatKind = ExportFormat["kind"];

export interface ExportOptions {
  format: ExportFormat;
  binary: BinaryEncoding;
}

export type ExportSource =
  /** 整张表,或表数据页当前筛选条件下的全部结果。 */
  | { kind: "table"; table: TableRef; filter: string | null; sort: SortSpec | null }
  /** 结果太大、后端没存全:在查询标签页自己的连接上重新执行,读出全部结果。 */
  | { kind: "query"; editorId: string; scope: QueryScope; sql: string }
  /** 后端存着的完整结果:直接写出来,`sort` 是表格上点的排序。 */
  | { kind: "result"; editorId: string; resultId: number; columns: ResultColumn[]; sort: ResultSort | null };

export interface ExportRequest {
  connectionId: string;
  source: ExportSource;
  options: ExportOptions;
  path: string;
}

/** 转储 SQL 文件:建表语句,withData 时连同全部数据的 INSERT。 */
export interface DumpRequest {
  connectionId: string;
  table: TableRef;
  withData: boolean;
  path: string;
}

export interface ExportOutcome {
  rows: number;
  bytes: number;
  path: string;
  /** 准备连接时发生的事(断线重连过)。 */
  notice: string | null;
}

export type TransferPhase = "preparing" | "exporting" | "checking" | "importing" | "finishing";

/** 后端经 transfer-progress 事件推来的进度。 */
export interface TransferProgress {
  jobId: string;
  phase: TransferPhase;
  rows: number;
  failed: number;
  /** 已知总量时的完成比例(0–1)。 */
  fraction: number | null;
}

export type SourceFormat =
  /** encoding、delimiter 为 null 时自动探测。 */
  | { kind: "csv"; encoding: TextEncoding | null; delimiter: string | null; quote: string }
  /** sheet 为 null 时用第一个工作表。 */
  | { kind: "excel"; sheet: string | null };

export interface SourceSpec {
  path: string;
  /** null:按扩展名判断。 */
  format: SourceFormat | null;
  hasHeader: boolean;
}

export interface SourcePreview {
  /** 探测之后实际用的格式。 */
  format: SourceFormat;
  sheets: string[];
  headers: string[];
  /** 前 20 行,null 是 NULL。 */
  rows: (string | null)[][];
  rowNumbers: number[];
}

/** 目标列 column 取来源第 source 列;null 是不导入。 */
export interface ColumnMapping {
  column: string;
  source: number | null;
}

/** 表上的主键(name 为 null)或一个唯一索引:「追加或更新」只能按它们匹配。 */
export interface UniqueKey {
  name: string | null;
  columns: string[];
}

export interface ImportPreview {
  source: SourcePreview;
  columns: ColumnInfo[];
  /** 主键在前。 */
  uniqueKeys: UniqueKey[];
  mapping: ColumnMapping[];
}

export type OnError = "stop" | "skip";

/** 追加、更新、追加或更新、删除、复制(先清空表再追加)。 */
export type ImportMode = "append" | "update" | "upsert" | "delete" | "replace";

export interface ImportOptions {
  onError: OnError;
  batchSize: number;
  mode: ImportMode;
  /** 匹配键(列名):更新、追加或更新、删除用。 */
  keyColumns: string[];
  binary: BinaryEncoding;
}

export interface ImportRequest {
  connectionId: string;
  table: TableRef;
  source: SourceSpec;
  mapping: ColumnMapping[];
  options: ImportOptions;
}

export type ImportStatus = "completed" | "stopped" | "cancelled" | "failed";

export interface ImportFailure {
  /** 文件里的行号。 */
  row: number;
  column: string | null;
  value: string | null;
  reason: string;
}

export interface ImportReport {
  status: ImportStatus;
  /** 已经生效的表行数:插入、更新或删除了多少行。 */
  affected: number;
  /** 更新、删除时在表里找不到对应行的文件行数。 */
  unmatched: number;
  failed: number;
  /** 最多带回 10000 条明细。 */
  failures: ImportFailure[];
  summary: string;
}
