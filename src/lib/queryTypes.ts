// SQL 编辑器:与 crates/basalt-core/src/query 和 src-tauri/src/service/query*.rs 的序列化结构对应。
import type { CellValue, TableRef } from "./types";

/** 查询标签页所在的库;schema 只有 PostgreSQL 用。 */
export interface QueryScope {
  database: string | null;
  schema: string | null;
}

/** 结果列直接取自哪张表的哪一列。 */
export interface ColumnSource {
  table: TableRef;
  /** 表里的列名(结果里可能起了别名)。 */
  column: string;
}

export interface ResultColumn {
  name: string;
  /** 驱动报的类型名(小写);认不出时为空串。 */
  dataType: string;
  /** 表达式、聚合、驱动说不清的为 null。 */
  source: ColumnSource | null;
}

export interface ResultSet {
  /** 完整结果存在后端,往下滚、排序时按它去取。 */
  id: number;
  columns: ResultColumn[];
  /** 第一批行;其余的用 resultRows 取。 */
  rows: CellValue[][];
  /** 后端存着的总行数。 */
  totalRows: number;
  /** 结果太大,后端只存了前 totalRows 行。 */
  truncated: boolean;
  /** 列的来源看得出、却不能在结果里改的原因,如来自临时表。 */
  readOnly: string | null;
}

/** 按第几列排序(结果里可能有同名列,所以用序号)。 */
export interface ResultSort {
  column: number;
  descending: boolean;
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
  /** 执行过程中服务器发来的消息(PostgreSQL 的 RAISE NOTICE 之类),按到达的先后。 */
  messages: ServerMessage[];
  /** 出错时错在语句的哪里:第几个 UTF-16 单元(从 0 数,和编辑器里的位置一样数),认不出时为 null。 */
  errorPosition: number | null;
}

export interface ServerMessage {
  /** NOTICE、WARNING、INFO、DEBUG。 */
  level: string;
  text: string;
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
