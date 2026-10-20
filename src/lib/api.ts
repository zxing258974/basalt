import type { ImportCandidate, ImportReport, ImportSource } from "./importTypes";
import type {
  HistoryEntry,
  QueryImportCandidate,
  QueryImportReport,
  QueryScope,
  SavedQueryInfo,
  SavedQueryRef,
  StatementOutcome,
} from "./queryTypes";
import type {
  ApplyOutcome,
  ColumnInfo,
  ConnectionConfig,
  ConnectionDraft,
  ConnectionView,
  PageRequest,
  RowChange,
  ServerInfo,
  TableInfo,
  TablePage,
  TableRef,
} from "./types";

/** 前端能调用的全部后端能力。Tauri 与 mock 后端都实现它。 */
export interface Backend {
  listConnections(): Promise<ConnectionView[]>;
  saveConnection(draft: ConnectionDraft): Promise<ConnectionConfig>;
  /** 连接成功后记住临时输入的密码;不断开会话。 */
  rememberPassword(id: string, password: string): Promise<void>;
  deleteConnection(id: string): Promise<void>;
  duplicateConnection(id: string): Promise<ConnectionConfig>;
  testConnection(draft: ConnectionDraft): Promise<ServerInfo>;
  openConnection(id: string, password?: string): Promise<ServerInfo>;
  closeConnection(id: string): Promise<void>;
  listDatabases(id: string): Promise<string[]>;
  listSchemas(id: string, database: string): Promise<string[]>;
  listTables(id: string, database: string, schema?: string | null): Promise<TableInfo[]>;
  readTablePage(id: string, table: TableRef, request: PageRequest): Promise<TablePage>;
  /** 将要执行的语句(参数内联成字面量),提交前给用户确认。 */
  previewTableChanges(id: string, table: TableRef, changes: RowChange[]): Promise<string[]>;
  /** 一个事务里提交,任何一条失败都整体回滚。`expected` 是用户确认过的预览,后端生成的不一样就拒绝。 */
  applyTableChanges(id: string, table: TableRef, changes: RowChange[], expected: string[]): Promise<ApplyOutcome>;
  pickSqliteFile(): Promise<string | null>;
  /** 扫描导入来源,给出预览;不写任何东西。 */
  scanImportSource(source: ImportSource): Promise<ImportCandidate[]>;
  /** 导入勾选的连接:密码由后端直接写进钥匙串。 */
  importConnections(source: ImportSource, ids: string[]): Promise<ImportReport>;
  pickNcxFile(): Promise<string | null>;
  /** 在查询标签页 `editorId` 的专用连接上执行一条语句;数据库报错也是一个结果,不会 reject。 */
  runStatement(editorId: string, connectionId: string, scope: QueryScope, sql: string): Promise<StatementOutcome>;
  /** 取消标签页正在执行的语句;没有在执行时返回 false。 */
  cancelStatement(editorId: string): Promise<boolean>;
  /** 标签页关了,断开它的专用连接。 */
  closeEditor(editorId: string): Promise<void>;
  /** 补全用的列名。 */
  listColumns(id: string, table: TableRef): Promise<ColumnInfo[]>;
  searchHistory(query: string, connectionId: string | null, limit: number): Promise<HistoryEntry[]>;
  clearHistory(): Promise<void>;
  listSavedQueries(connectionId: string, database: string, schema: string | null): Promise<SavedQueryInfo[]>;
  readSavedQuery(query: SavedQueryRef): Promise<string>;
  /** 同名的已经有了且没让覆盖时报 already_exists。 */
  saveQuery(query: SavedQueryRef, sql: string, overwrite: boolean): Promise<SavedQueryInfo>;
  deleteSavedQuery(query: SavedQueryRef): Promise<void>;
  /** 扫描本机 Navicat 保存的查询,给出预览;不写任何东西。 */
  scanNavicatQueries(): Promise<QueryImportCandidate[]>;
  importNavicatQueries(ids: string[]): Promise<QueryImportReport>;
}

function createTauriBackend(): Backend {
  // 动态导入,浏览器里跑 mock 时不会加载 Tauri 运行时。
  const call = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(command, args);
  };
  return {
    listConnections: () => call("list_connections"),
    saveConnection: (draft) => call("save_connection", { draft }),
    rememberPassword: (id, password) => call("remember_password", { id, password }),
    deleteConnection: (id) => call("delete_connection", { id }),
    duplicateConnection: (id) => call("duplicate_connection", { id }),
    testConnection: (draft) => call("test_connection", { draft }),
    openConnection: (id, password) => call("open_connection", { id, password: password ?? null }),
    closeConnection: (id) => call("close_connection", { id }),
    listDatabases: (id) => call("list_databases", { id }),
    listSchemas: (id, database) => call("list_schemas", { id, database }),
    listTables: (id, database, schema) => call("list_tables", { id, database, schema: schema ?? null }),
    readTablePage: (id, table, request) => call("read_table_page", { id, table, request }),
    previewTableChanges: (id, table, changes) => call("preview_table_changes", { id, table, changes }),
    applyTableChanges: (id, table, changes, expected) => call("apply_table_changes", { id, table, changes, expected }),
    pickSqliteFile: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "SQLite", extensions: ["db", "sqlite", "sqlite3", "db3"] }],
      });
      return typeof picked === "string" ? picked : null;
    },
    scanImportSource: (source) => call("scan_import_source", { source }),
    importConnections: (source, ids) => call("import_connections", { source, ids }),
    pickNcxFile: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Navicat 导出的连接", extensions: ["ncx"] }],
      });
      return typeof picked === "string" ? picked : null;
    },
    runStatement: (editorId, connectionId, scope, sql) => call("run_statement", { editorId, connectionId, scope, sql }),
    cancelStatement: (editorId) => call("cancel_statement", { editorId }),
    closeEditor: (editorId) => call("close_editor", { editorId }),
    listColumns: (id, table) => call("list_columns", { id, table }),
    searchHistory: (query, connectionId, limit) => call("search_history", { query, connectionId, limit }),
    clearHistory: () => call("clear_history"),
    listSavedQueries: (connectionId, database, schema) =>
      call("list_saved_queries", { connectionId, database, schema }),
    readSavedQuery: (query) => call("read_saved_query", { query }),
    saveQuery: (query, sql, overwrite) => call("save_query", { query, sql, overwrite }),
    deleteSavedQuery: (query) => call("delete_saved_query", { query }),
    scanNavicatQueries: () => call("scan_navicat_queries"),
    importNavicatQueries: (ids) => call("import_navicat_queries", { ids }),
  };
}

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

let backend: Backend | null = null;

export async function getBackend(): Promise<Backend> {
  if (backend) return backend;
  if (isTauriRuntime()) {
    backend = createTauriBackend();
  } else {
    const { createMockBackend } = await import("./mockBackend");
    backend = createMockBackend();
  }
  return backend;
}

/** 测试里注入假的后端。 */
export function setBackendForTesting(next: Backend | null): void {
  backend = next;
}
