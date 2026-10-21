// mock 后端的导入导出:按示例表的列造预览,一步一步推进度,能取消。行为贴近 Rust 端。
import type { Backend } from "../api";
import type { ImportFailure, ImportReport, SourcePreview, TransferPhase, TransferProgress } from "../transferTypes";
import type { CommandError, ColumnInfo, TableRef } from "../types";
import { seedTable } from "./seedTables";

type TransferApi = Pick<
  Backend,
  | "pickSavePath"
  | "pickImportFile"
  | "exportData"
  | "previewImport"
  | "importData"
  | "cancelTransfer"
  | "onTransferProgress"
  | "saveImportFailures"
  | "revealInFinder"
>;

/** 示例表在 mock 里假装有多少行,让进度条看得见。 */
const PRETEND_ROWS = 48_000;
const STEPS = 12;

function cancelled(): never {
  const error: CommandError = { code: "cancelled", message: "已取消", fields: [] };
  throw error;
}

/** 按列类型造一个示例值,预览里显示。 */
function sample(column: ColumnInfo, row: number): string | null {
  const type = column.dataType.toLowerCase();
  if (column.nullable && row % 7 === 3) return null;
  if (type.includes("int")) return String(row + 1);
  if (type.includes("decimal") || type.includes("numeric")) return (row * 12.5 + 3).toFixed(2);
  if (type.includes("date"))
    return `2026-10-${String((row % 28) + 1).padStart(2, "0")} 09:${String(row % 60).padStart(2, "0")}:00`;
  return `${column.name} ${row + 1}`;
}

function preview(path: string, columns: ColumnInfo[], hasHeader: boolean, sheet: string | null): SourcePreview {
  const excel = /\.xlsx?$/i.test(path);
  // 文件里少最后一列,第二列的列名写法不一样:看得出自动匹配怎么处理。
  const fileColumns = columns.slice(0, Math.max(1, columns.length - 1));
  const headers = fileColumns.map((c, i) => (i === 1 ? c.name.toUpperCase() : c.name));
  const rows = Array.from({ length: 20 }, (_, row) => fileColumns.map((c) => sample(c, row)));
  return {
    format: excel
      ? { kind: "excel", sheet: sheet ?? "Sheet1" }
      : { kind: "csv", encoding: "utf8", delimiter: ",", quote: '"' },
    sheets: excel ? ["Sheet1", "说明"] : [],
    headers: hasHeader ? headers : headers.map((_, i) => `第 ${i + 1} 列`),
    rows,
    rowNumbers: rows.map((_, i) => i + (hasHeader ? 2 : 1)),
  };
}

export function createMockTransfer(wait: (ms?: number) => Promise<unknown>): TransferApi {
  const listeners = new Set<(progress: TransferProgress) => void>();
  const running = new Map<string, { cancelled: boolean }>();
  const emit = (progress: TransferProgress) => listeners.forEach((listener) => listener(progress));

  /** 一步一步推进度;中途取消就以 cancelled 结束。 */
  async function simulate(jobId: string, phase: TransferPhase, total: number): Promise<void> {
    const job = { cancelled: false };
    running.set(jobId, job);
    try {
      emit({ jobId, phase: "preparing", rows: 0, failed: 0, fraction: null });
      for (let step = 1; step <= STEPS; step += 1) {
        await wait(60);
        if (job.cancelled) cancelled();
        const rows = Math.round((total * step) / STEPS);
        emit({ jobId, phase, rows, failed: 0, fraction: phase === "importing" ? rows / total : null });
      }
      emit({ jobId, phase: "finishing", rows: total, failed: 0, fraction: 1 });
    } finally {
      running.delete(jobId);
    }
  }

  const tableRows = (table: TableRef) => Math.max(seedTable(table.table).rows.length, PRETEND_ROWS);

  return {
    async pickSavePath(defaultName) {
      await wait(60);
      return `/Users/me/Downloads/${defaultName}`;
    },
    async pickImportFile() {
      await wait(60);
      return "/Users/me/Downloads/customers.csv";
    },
    async exportData(jobId, request) {
      const { source } = request;
      const rows =
        source.kind === "rows" ? source.rows.length : source.kind === "table" ? tableRows(source.table) : 12_480;
      await simulate(jobId, "exporting", rows);
      return { rows, bytes: rows * 96 + 64, path: request.path, notice: null };
    },
    async previewImport(_connectionId, table, source) {
      await wait();
      const columns = seedTable(table.table).columns;
      const sheet = source.format?.kind === "excel" ? source.format.sheet : null;
      const file = preview(source.path, columns, source.hasHeader, sheet);
      const mapping = columns
        .filter((c) => !c.isGenerated)
        .map((column) => {
          const index = file.headers.findIndex((h) => h.toLowerCase() === column.name.toLowerCase());
          return { column: column.name, source: index >= 0 ? index : null };
        });
      return { source: file, columns, mapping };
    },
    async importData(jobId, request) {
      const total = 1_280;
      try {
        await simulate(jobId, "importing", total);
      } catch (error) {
        const report: ImportReport = {
          status: "cancelled",
          inserted: 0,
          failed: 0,
          failures: [],
          summary: "已取消。整个导入已经回滚，表没有变化。",
        };
        if ((error as CommandError).code === "cancelled") return report;
        throw error;
      }
      const failures: ImportFailure[] =
        request.options.onError === "skip"
          ? [
              { row: 18, column: request.mapping[0]?.column ?? null, value: "4x2", reason: "不是有效的整数" },
              { row: 205, column: null, value: null, reason: "Duplicate entry '205' for key 'PRIMARY'" },
            ]
          : [];
      const inserted = total - failures.length;
      const cleared = request.options.clearTable ? "导入前清空了表（删掉 12 行）。" : "";
      const tail = failures.length ? `，${failures.length} 行失败（见下面的明细）` : "";
      return {
        status: "completed",
        inserted,
        failed: failures.length,
        failures,
        summary: `${cleared}导入完成：成功 ${inserted} 行${tail}。`,
      };
    },
    cancelTransfer(jobId) {
      const job = running.get(jobId);
      if (job) job.cancelled = true;
      return Promise.resolve(job !== undefined);
    },
    onTransferProgress(handler) {
      listeners.add(handler);
      return Promise.resolve(() => void listeners.delete(handler));
    },
    async saveImportFailures(_path, failures) {
      await wait(60);
      return 32 + failures.length * 48;
    },
    async revealInFinder() {
      await wait(20);
    },
  };
}
