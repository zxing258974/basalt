// SQL 编辑器:与 crates/basalt-core/src/query 和 src-tauri/src/service/query*.rs 的序列化结构对应。
import type { CellValue } from "./types";

/** 查询标签页所在的库;schema 只有 PostgreSQL 用。 */
export interface QueryScope {
  database: string | null;
  schema: string | null;
}

export interface ResultColumn {
  name: string;
  /** 驱动报的类型名(小写);认不出时为空串。 */
  dataType: string;
}

export interface ResultSet {
  columns: ResultColumn[];
  rows: CellValue[][];
  /** 超出上限的行没有带回来。 */
  truncated: boolean;
}

export type OutcomeStatus = "ok" | "error" | "cancelled";

export interface StatementOutcome {
  status: OutcomeStatus;
  /** 出错或取消时数据库给的原文。 */
  message: string | null;
  resultSets: ResultSet[];
  /** 增删改的影响行数;查询、DDL 为 null。 */
  affectedRows: number | null;
  elapsedMs: number;
  /** 执行后连接所在的库:USE、SET search_path 改了它时标签页跟着改。 */
  scope: QueryScope;
  /** 执行前发生的事,如断线重连。 */
  notice: string | null;
}

export interface HistoryEntry {
  id: string;
  executedAt: string;
  connectionId: string;
  connectionName: string;
  database: string | null;
  schema: string | null;
  sql: string;
  status: OutcomeStatus;
  elapsedMs: number;
  affectedRows: number | null;
  rowCount: number | null;
}

/** 一条保存的查询在哪里。PostgreSQL 的挂在 schema 下。 */
export interface SavedQueryRef {
  connectionId: string;
  database: string;
  schema: string | null;
  name: string;
}

export interface SavedQueryInfo {
  name: string;
  modifiedAt: string | null;
}

export interface QueryTarget {
  connectionId: string;
  connectionName: string;
  database: string;
  schema: string | null;
  /** 存到 Basalt 时用的名字。 */
  name: string;
}

export type QueryImportStatus =
  | { kind: "new" }
  /** 目标位置已有内容一样的同名查询。 */
  | { kind: "exists" }
  /** 目标位置已有同名、内容不同的查询,不覆盖。 */
  | { kind: "conflict" }
  | { kind: "unmatched"; reason: string };

export interface QueryImportCandidate {
  /** 相对 Navicat 查询目录的路径,导入时按它回传勾选项。 */
  id: string;
  name: string;
  sourceType: string;
  sourceConnection: string;
  sourceDatabase: string;
  sourceSchema: string | null;
  target: QueryTarget | null;
  matchedBy: "name" | "address" | null;
  status: QueryImportStatus;
}

export interface QueryImportReport {
  imported: { name: string; target: QueryTarget }[];
  skipped: { name: string; reason: string }[];
}
