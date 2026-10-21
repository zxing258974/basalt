import type { DdlOutcome, DdlPreview, TableDesign } from "./designTypes";
import type { ImportCandidate, ImportReport, ImportSource } from "./importTypes";
import type {
  ExportOutcome,
  ExportRequest,
  ImportFailure,
  ImportPreview,
  ImportReport as DataImportReport,
  ImportRequest,
  SourceSpec,
  TransferProgress,
} from "./transferTypes";
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
  CopyTableOutcome,
  TableDetails,
  DeletedConnection,
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
  /** 不会弹钥匙串授权框;删不掉的密码留在钥匙串里,由 passwordLeft 报告。 */
  deleteConnection(id: string): Promise<DeletedConnection>;
  duplicateConnection(id: string): Promise<ConnectionConfig>;
  testConnection(draft: ConnectionDraft): Promise<ServerInfo>;
  openConnection(id: string, password?: string): Promise<ServerInfo>;
  closeConnection(id: string): Promise<void>;
  listDatabases(id: string): Promise<string[]>;
  listSchemas(id: string, database: string): Promise<string[]>;
  listTables(id: string, database: string, schema?: string | null): Promise<TableInfo[]>;
  readTablePage(id: string, table: TableRef, request: PageRequest): Promise<TablePage>;
  /** DROP TABLE;被外键、视图依赖时数据库会拒绝,原样报错。 */
  dropTable(id: string, table: TableRef): Promise<void>;
  /** 复制到同一个库(schema)里;withData 为假时只复制结构。 */
  copyTable(id: string, table: TableRef, name: string, withData: boolean): Promise<CopyTableOutcome>;
  /** 信息面板:表或视图的属性和建表语句。 */
  tableDetails(id: string, table: TableRef): Promise<TableDetails>;
  /** 将要执行的语句(参数内联成字面量),提交前给用户确认。 */
  previewTableChanges(id: string, table: TableRef, changes: RowChange[]): Promise<string[]>;
  /** 一个事务里提交,任何一条失败都整体回滚。`expected` 是用户确认过的预览,后端生成的不一样就拒绝。 */
  applyTableChanges(id: string, table: TableRef, changes: RowChange[], expected: string[]): Promise<ApplyOutcome>;
  readTableDesign(id: string, table: TableRef): Promise<TableDesign>;
  /** 将要执行的 DDL;`original` 是要改的表,新建表时为 null。 */
  previewTableDesign(id: string, original: TableRef | null, design: TableDesign): Promise<DdlPreview>;
  /** `expected` 是用户确认过的预览,后端重新生成的不一样就拒绝执行。 */
  applyTableDesign(id: string, original: TableRef | null, design: TableDesign, expected: string[]): Promise<DdlOutcome>;
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
  /** 告诉后端现在有没有未提交的表格修改、没保存的查询:有的话 ⌘Q、关窗口会先拦下来问前端。 */
  setUnsavedWork(unsaved: boolean): Promise<void>;
  /** 用户按了 ⌘Q 或关主窗口、后端拦下来了;返回取消监听的函数。 */
  onQuitRequested(handler: () => void): Promise<() => void>;
  /** 不再检查,直接退出应用。 */
  quitApp(): Promise<void>;
  /** 保存对话框:导出、存失败明细时选路径。用户取消返回 null。 */
  pickSavePath(defaultName: string, filter: FileFilter): Promise<string | null>;
  /** 选要导入的 CSV / Excel 文件。 */
  pickImportFile(): Promise<string | null>;
  /** 导出;`jobId` 由前端起,命令还没返回时也能按它取消、认领进度。取消时以 code `cancelled` reject。 */
  exportData(jobId: string, request: ExportRequest): Promise<ExportOutcome>;
  /** 读文件预览、目标表的列和自动匹配的映射。 */
  previewImport(connectionId: string, table: TableRef, source: SourceSpec): Promise<ImportPreview>;
  /** 导入;数据库层面的失败写在报告里,不 reject。 */
  importData(jobId: string, request: ImportRequest): Promise<DataImportReport>;
  /** 取消导入导出;任务已经结束时返回 false。 */
  cancelTransfer(jobId: string): Promise<boolean>;
  /** 导入导出的进度;返回取消监听的函数。 */
  onTransferProgress(handler: (progress: TransferProgress) => void): Promise<() => void>;
  saveImportFailures(path: string, failures: ImportFailure[]): Promise<number>;
  /** 在访达里选中这个文件。 */
  revealInFinder(path: string): Promise<void>;
}

/** 文件对话框里的类型筛选。 */
export interface FileFilter {
  name: string;
  extensions: string[];
}

/** 后端拦下退出时发给前端的事件,和 src-tauri/src/quit.rs 一致。 */
const QUIT_REQUESTED_EVENT = "quit-requested";
/** 导入导出的进度事件,和 src-tauri/src/commands.rs 一致。 */
const TRANSFER_PROGRESS_EVENT = "transfer-progress";

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
    dropTable: (id, table) => call("drop_table", { id, table }),
    copyTable: (id, table, name, withData) => call("copy_table", { id, table, name, withData }),
    tableDetails: (id, table) => call("table_details", { id, table }),
    readTablePage: (id, table, request) => call("read_table_page", { id, table, request }),
    previewTableChanges: (id, table, changes) => call("preview_table_changes", { id, table, changes }),
    applyTableChanges: (id, table, changes, expected) => call("apply_table_changes", { id, table, changes, expected }),
    readTableDesign: (id, table) => call("read_table_design", { id, table }),
    previewTableDesign: (id, original, design) => call("preview_table_design", { id, original, design }),
    applyTableDesign: (id, original, design, expected) =>
      call("apply_table_design", { id, original, design, expected }),
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
    setUnsavedWork: (unsaved) => call("set_unsaved_work", { unsaved }),
    onQuitRequested: async (handler) => {
      const { listen } = await import("@tauri-apps/api/event");
      return listen(QUIT_REQUESTED_EVENT, () => handler());
    },
    quitApp: () => call("quit_app"),
    pickSavePath: async (defaultName, filter) => {
      const { save } = await import("@tauri-apps/plugin-dialog");
      return (await save({ defaultPath: defaultName, filters: [filter] })) ?? null;
    },
    pickImportFile: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "CSV 或 Excel", extensions: ["csv", "tsv", "txt", "xlsx", "xlsm", "xls"] }],
      });
      return typeof picked === "string" ? picked : null;
    },
    exportData: (jobId, request) => call("export_data", { jobId, request }),
    previewImport: (connectionId, table, source) => call("preview_import", { connectionId, table, source }),
    importData: (jobId, request) => call("import_data", { jobId, request }),
    cancelTransfer: (jobId) => call("cancel_transfer", { jobId }),
    onTransferProgress: async (handler) => {
      const { listen } = await import("@tauri-apps/api/event");
      return listen<TransferProgress>(TRANSFER_PROGRESS_EVENT, (event) => handler(event.payload));
    },
    saveImportFailures: (path, failures) => call("save_import_failures", { path, failures }),
    revealInFinder: (path) => call("reveal_in_finder", { path }),
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
