import type { DdlOutcome, DdlPreview, TableDesign } from "./designTypes";
import type {
  ConsoleOutcome,
  KeyDetail,
  KeyName,
  NewKey,
  RedisKeyspace,
  RedisKind,
  ScanPage,
  ScanRequest,
} from "./redisTypes";
import type { ImportCandidate, ImportReport, ImportSource } from "./importTypes";
import type { KillTarget, ServerSnapshot } from "./monitorTypes";
import type {
  ChatDelta,
  ChatOutcome,
  ChatRequest,
  LlmModel,
  LlmModelDraft,
  LlmSettings,
  OutlineTable,
  TestReply,
} from "./llmTypes";
import type {
  DumpRequest,
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
  ResultSort,
  SavedQueryInfo,
  SavedQueryRef,
  StatementOutcome,
} from "./queryTypes";
import type {
  ApplyOutcome,
  CellValue,
  ClearMode,
  ClearTableOutcome,
  ColumnInfo,
  ConnectionConfig,
  ConnectionDraft,
  ConnectionView,
  CopyTableOutcome,
  TableDetails,
  DeletedConnection,
  HostKey,
  Maintenance,
  MaintenanceOutcome,
  PageRequest,
  ReferencingKey,
  RowChange,
  ServerInfo,
  SshConfigHost,
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
  /** 同上,记住 SSH 密码或私钥口令。 */
  rememberSshSecret(id: string, secret: string): Promise<void>;
  /** 用户核对过指纹:记下这台 SSH 服务器的主机密钥。 */
  trustHostKey(hostKey: HostKey): Promise<void>;
  /** ~/.ssh/config 里写明了名字的主机;没有这个文件时是空的。 */
  listSshConfigHosts(): Promise<SshConfigHost[]>;
  /** 不会弹钥匙串授权框;删不掉的密码留在钥匙串里,由 passwordLeft 报告。 */
  deleteConnection(id: string): Promise<DeletedConnection>;
  duplicateConnection(id: string): Promise<ConnectionConfig>;
  testConnection(draft: ConnectionDraft): Promise<ServerInfo>;
  /** password、sshSecret 只在没保存时传(用户在密码框里输的)。 */
  openConnection(id: string, password?: string, sshSecret?: string): Promise<ServerInfo>;
  closeConnection(id: string): Promise<void>;
  listDatabases(id: string): Promise<string[]>;
  /** Redis 连接下有哪些库、各有多少键。 */
  redisKeyspace(id: string): Promise<RedisKeyspace>;
  /** 按模式和类型扫描一批键。 */
  redisScanKeys(id: string, db: number, request: ScanRequest): Promise<ScanPage>;
  redisReadKey(id: string, db: number, key: string): Promise<KeyDetail>;
  /** hash、list、set、zset 表格里的修改 → 给人看的命令。 */
  redisPreviewKeyChanges(id: string, key: string, kind: RedisKind, changes: RowChange[]): Promise<string[]>;
  /** 核对后一起写入;`expected` 是看过的预览,重新生成的不一样就不执行。 */
  redisApplyKeyChanges(
    id: string,
    db: number,
    key: string,
    kind: RedisKind,
    changes: RowChange[],
    expected: string[],
  ): Promise<ApplyOutcome>;
  /** 改字符串的值;`original` 是读出来时的值,被别人改过就不覆盖。 */
  redisSetString(id: string, db: number, key: string, original: string, value: string): Promise<void>;
  redisCreateKey(id: string, db: number, key: NewKey): Promise<KeyName>;
  /** 新名字已经有键时报 already_exists,不覆盖。 */
  redisRenameKey(id: string, db: number, key: string, name: string): Promise<KeyName>;
  redisDeleteKeys(id: string, db: number, keys: string[]): Promise<number>;
  /** `seconds` 为 null 时不过期。 */
  redisSetTtl(id: string, db: number, key: string, seconds: number | null): Promise<void>;
  /** 在命令行标签页 `console` 里执行一行;危险的命令要 `confirmed` 才执行。`db` 是界面上的库,重连时按它连。 */
  redisConsoleRun(id: string, console: string, db: number, line: string, confirmed: boolean): Promise<ConsoleOutcome>;
  /** 停下正在跑的命令(断开这个命令行的连接,下一条命令重连)。 */
  redisConsoleCancel(id: string, console: string): Promise<void>;
  redisConsoleClose(id: string, console: string): Promise<void>;
  /** 服务器认识的命令名,大写、排好序(补全用)。 */
  redisCommandNames(id: string): Promise<string[]>;
  listSchemas(id: string, database: string): Promise<string[]>;
  listTables(id: string, database: string, schema?: string | null): Promise<TableInfo[]>;
  readTablePage(id: string, table: TableRef, request: PageRequest): Promise<TablePage>;
  /** DROP TABLE;被外键、视图依赖时数据库会拒绝,原样报错。 */
  dropTable(id: string, table: TableRef): Promise<void>;
  /** 复制到同一个库(schema)里;withData 为假时只复制结构。 */
  copyTable(id: string, table: TableRef, name: string, withData: boolean): Promise<CopyTableOutcome>;
  /** 清空(DELETE)或截断(TRUNCATE);被外键引用时截断会被数据库拒绝,原样报错。 */
  clearTable(id: string, table: TableRef, mode: ClearMode): Promise<ClearTableOutcome>;
  /** 改名,留在同一个库(schema)里;返回新表。 */
  renameTable(id: string, table: TableRef, name: string): Promise<TableRef>;
  /** 别的表上引用这张表的外键:清空、截断的确认框写明会波及哪些表。 */
  tableReferences(id: string, table: TableRef): Promise<ReferencingKey[]>;
  /** 维护;这种数据库没有这项时以 unsupported reject。 */
  maintainTable(id: string, table: TableRef, operation: Maintenance): Promise<MaintenanceOutcome>;
  /** 信息面板:表或视图的属性和建表语句。 */
  tableDetails(id: string, table: TableRef): Promise<TableDetails>;
  /** 服务器监控:会话列表和累计计数器;SQLite 以 unsupported reject。 */
  serverSnapshot(id: string): Promise<ServerSnapshot>;
  /** 终止一个会话:只停语句,或断开连接。会话已经不是确认时的样子就 reject。 */
  killProcess(id: string, target: KillTarget): Promise<void>;
  /** SQL 助手:模型配置。API Key 在密码存储里,不随列表返回。 */
  listLlmModels(): Promise<LlmSettings>;
  saveLlmModel(draft: LlmModelDraft): Promise<LlmModel>;
  deleteLlmModel(id: string): Promise<LlmSettings>;
  setActiveLlmModel(id: string): Promise<LlmSettings>;
  /** 用表单里的设置(没保存也行)发一句很短的话。 */
  testLlmModel(draft: LlmModelDraft): Promise<TestReply>;
  /** 流式对话:回答一段段经 onLlmChat 推来;返回时已经完整。被取消时以 cancelled 错误结束。 */
  llmChat(requestId: string, request: ChatRequest): Promise<ChatOutcome>;
  cancelLlmChat(requestId: string): Promise<boolean>;
  onLlmChat(handler: (delta: ChatDelta) => void): Promise<() => void>;
  /** 给助手的表结构概要(表名、列名、类型),不含数据。 */
  schemaOutline(id: string, database: string, schema: string | null): Promise<OutlineTable[]>;
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
  /** 选 SSH 私钥(从 ~/.ssh 开始)或证书文件。 */
  pickCredentialFile(kind: CredentialFile): Promise<string | null>;
  /** 扫描导入来源,给出预览;不写任何东西。 */
  scanImportSource(source: ImportSource): Promise<ImportCandidate[]>;
  /** 导入勾选的连接:密码由后端直接写进密码存储。 */
  importConnections(source: ImportSource, ids: string[]): Promise<ImportReport>;
  pickNcxFile(): Promise<string | null>;
  /** 在查询标签页 `editorId` 的专用连接上执行一条语句;数据库报错也是一个结果,不会 reject。 */
  runStatement(editorId: string, connectionId: string, scope: QueryScope, sql: string): Promise<StatementOutcome>;
  /** 取消标签页正在执行的语句;没有在执行时返回 false。 */
  cancelStatement(editorId: string): Promise<boolean>;
  /** 取结果集里从 offset 起的 limit 行;给了排序就按排序后的顺序。 */
  resultRows(
    editorId: string,
    resultId: number,
    offset: number,
    limit: number,
    sort: ResultSort | null,
  ): Promise<CellValue[][]>;
  /** 不再显示的结果:让后端放掉它们占的内存。 */
  releaseResults(editorId: string, resultIds: number[]): Promise<void>;
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
  /** 上次开着的标签页(src/lib/workspace.ts 的格式);第一次启动、或记录读不懂时为 null。 */
  loadWorkspace(): Promise<string | null>;
  /** 记下现在开着的标签页,下次启动时恢复。 */
  saveWorkspace(content: string): Promise<void>;
  /** 告诉后端退出前要不要先问前端:有未提交的表格修改、没保存的表结构,或者标签页还没记下来。 */
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
  /** 转储 SQL 文件;和 exportData 一样按 jobId 认领进度、取消。 */
  dumpTable(jobId: string, request: DumpRequest): Promise<ExportOutcome>;
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

export type CredentialFile = "sshKey" | "certificate" | "key";

/** 文件对话框里的类型筛选。 */
export interface FileFilter {
  name: string;
  extensions: string[];
}

/** 助手回答的流式片段,和 src-tauri/src/commands.rs 一致。 */
export const LLM_CHAT_EVENT = "llm-chat";

/** 后端拦下退出时发给前端的事件,和 src-tauri/src/quit.rs 一致。 */
const QUIT_REQUESTED_EVENT = "quit-requested";
/** 导入导出的进度事件,和 src-tauri/src/commands.rs 一致。 */
const TRANSFER_PROGRESS_EVENT = "transfer-progress";

async function sshDirectory(): Promise<string | undefined> {
  try {
    const { homeDir, join } = await import("@tauri-apps/api/path");
    return await join(await homeDir(), ".ssh");
  } catch {
    return undefined;
  }
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
    rememberSshSecret: (id, secret) => call("remember_ssh_secret", { id, secret }),
    trustHostKey: (hostKey) => call("trust_host_key", { hostKey }),
    listSshConfigHosts: () => call("list_ssh_config_hosts"),
    deleteConnection: (id) => call("delete_connection", { id }),
    duplicateConnection: (id) => call("duplicate_connection", { id }),
    testConnection: (draft) => call("test_connection", { draft }),
    openConnection: (id, password, sshSecret) =>
      call("open_connection", { id, password: password ?? null, sshSecret: sshSecret ?? null }),
    closeConnection: (id) => call("close_connection", { id }),
    listDatabases: (id) => call("list_databases", { id }),
    redisKeyspace: (id) => call("redis_keyspace", { id }),
    redisScanKeys: (id, db, request) => call("redis_scan_keys", { id, db, request }),
    redisReadKey: (id, db, key) => call("redis_read_key", { id, db, key }),
    redisPreviewKeyChanges: (id, key, kind, changes) => call("redis_preview_key_changes", { id, key, kind, changes }),
    redisApplyKeyChanges: (id, db, key, kind, changes, expected) =>
      call("redis_apply_key_changes", { id, db, key, kind, changes, expected }),
    redisSetString: (id, db, key, original, value) => call("redis_set_string", { id, db, key, original, value }),
    redisCreateKey: (id, db, key) => call("redis_create_key", { id, db, key }),
    redisRenameKey: (id, db, key, name) => call("redis_rename_key", { id, db, key, name }),
    redisDeleteKeys: (id, db, keys) => call("redis_delete_keys", { id, db, keys }),
    redisSetTtl: (id, db, key, seconds) => call("redis_set_ttl", { id, db, key, seconds }),
    redisConsoleRun: (id, console, db, line, confirmed) =>
      call("redis_console_run", { id, console, db, line, confirmed }),
    redisConsoleCancel: (id, console) => call("redis_console_cancel", { id, console }),
    redisConsoleClose: (id, console) => call("redis_console_close", { id, console }),
    redisCommandNames: (id) => call("redis_command_names", { id }),
    listSchemas: (id, database) => call("list_schemas", { id, database }),
    listTables: (id, database, schema) => call("list_tables", { id, database, schema: schema ?? null }),
    dropTable: (id, table) => call("drop_table", { id, table }),
    copyTable: (id, table, name, withData) => call("copy_table", { id, table, name, withData }),
    clearTable: (id, table, mode) => call("clear_table", { id, table, mode }),
    renameTable: (id, table, name) => call("rename_table", { id, table, name }),
    maintainTable: (id, table, operation) => call("maintain_table", { id, table, operation }),
    tableReferences: (id, table) => call("table_references", { id, table }),
    tableDetails: (id, table) => call("table_details", { id, table }),
    serverSnapshot: (id) => call("server_snapshot", { id }),
    killProcess: (id, target) => call("kill_process", { id, target }),
    listLlmModels: () => call("list_llm_models"),
    saveLlmModel: (draft) => call("save_llm_model", { draft }),
    deleteLlmModel: (id) => call("delete_llm_model", { id }),
    setActiveLlmModel: (id) => call("set_active_llm_model", { id }),
    testLlmModel: (draft) => call("test_llm_model", { draft }),
    llmChat: (requestId, request) => call("llm_chat", { requestId, request }),
    cancelLlmChat: (requestId) => call("cancel_llm_chat", { requestId }),
    onLlmChat: async (handler) => {
      const { listen } = await import("@tauri-apps/api/event");
      return listen<ChatDelta>(LLM_CHAT_EVENT, (event) => handler(event.payload));
    },
    schemaOutline: (id, database, schema) => call("schema_outline", { id, database, schema }),
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
    pickCredentialFile: async (kind) => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      // 私钥通常没有扩展名,不加筛选;从 ~/.ssh 开始找。
      const defaultPath = kind === "sshKey" ? await sshDirectory() : undefined;
      const picked = await open({ multiple: false, directory: false, defaultPath });
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
    resultRows: (editorId, resultId, offset, limit, sort) =>
      call("result_rows", { editorId, resultId, offset, limit, sort }),
    releaseResults: (editorId, resultIds) => call("release_results", { editorId, resultIds }),
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
    loadWorkspace: () => call("load_workspace"),
    saveWorkspace: (content) => call("save_workspace", { content }),
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
    dumpTable: (jobId, request) => call("dump_table", { jobId, request }),
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
