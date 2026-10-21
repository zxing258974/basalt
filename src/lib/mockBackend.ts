// 浏览器里开发界面用的假后端:内存数据 + 少量延迟,行为尽量贴近 Rust 端。
import type { Backend } from "./api";
import { createMockQueries } from "./mock/queryMock";
import { seedTable } from "./mock/seedTables";
import { createMockTables } from "./mock/tableData";
import { createMockDesigns } from "./mock/tableDesign";
import { mockTableDetails } from "./mock/tableDetails";
import { createMockTransfer } from "./mock/transferMock";
import { classifyMockImport, mockImportSources, planMockImport } from "./mockImport";
import type {
  CommandError,
  ConnectionConfig,
  ConnectionDraft,
  DriverKind,
  Endpoint,
  ConnectionView,
  FieldError,
  ServerInfo,
  TableInfo,
} from "./types";

const LATENCY_MS = 120;

// 单测里不模拟延迟,省得每个用例都要等;浏览器里保留,能看到加载状态。
const wait = (ms = LATENCY_MS) =>
  import.meta.env.MODE === "test" ? Promise.resolve() : new Promise((resolve) => setTimeout(resolve, ms));

function fail(code: string, message: string, fields: FieldError[] = []): never {
  const error: CommandError = { code, message, fields };
  throw error;
}

function table(name: string, partial: Partial<TableInfo> = {}): TableInfo {
  return {
    name,
    kind: "table",
    engine: null,
    rows: null,
    dataBytes: null,
    indexBytes: null,
    createdAt: null,
    updatedAt: null,
    comment: null,
    ...partial,
  };
}

const SEED_CONNECTIONS: ConnectionConfig[] = [
  {
    id: "mock-mysql",
    name: "本地 MySQL",
    group: "开发",
    driver: "mysql",
    endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root" },
    savePassword: true,
  },
  {
    id: "mock-pg",
    name: "FOLIO 测试库",
    group: "开发",
    driver: "postgres",
    endpoint: { kind: "network", host: "10.0.8.21", port: 5432, username: "folio", database: "folio" },
    savePassword: false,
  },
  {
    id: "mock-sqlite",
    name: "笔记",
    driver: "sqlite",
    endpoint: { kind: "file", path: "/Users/me/Documents/notes.db" },
    savePassword: false,
  },
];

const SERVERS: Record<DriverKind, ServerInfo> = {
  mysql: { product: "MySQL", version: "8.4.3" },
  mariadb: { product: "MariaDB", version: "11.4.4-MariaDB" },
  postgres: { product: "PostgreSQL", version: "17.11" },
  sqlite: { product: "SQLite", version: "3.46.0" },
};

const MYSQL_TABLES: Record<string, TableInfo[]> = {
  shop: [
    table("customers", {
      engine: "InnoDB",
      rows: 18_204,
      dataBytes: 2_637_824,
      indexBytes: 540_672,
      createdAt: "2026-03-02 10:12:40",
      updatedAt: "2026-10-19 22:41:03",
      comment: "客户",
    }),
    table("order_items", {
      engine: "InnoDB",
      rows: 412_880,
      dataBytes: 39_337_984,
      indexBytes: 12_075_008,
      createdAt: "2026-03-02 10:12:41",
      updatedAt: "2026-10-20 09:58:12",
    }),
    table("orders", {
      engine: "InnoDB",
      rows: 96_310,
      dataBytes: 11_026_432,
      indexBytes: 4_734_976,
      createdAt: "2026-03-02 10:12:41",
      updatedAt: "2026-10-20 09:58:12",
      comment: "订单主表",
    }),
    table("products", {
      engine: "InnoDB",
      rows: 1_204,
      dataBytes: 344_064,
      indexBytes: 49_152,
      createdAt: "2026-03-02 10:12:40",
      comment: "商品",
    }),
    table("v_daily_sales", { kind: "view" }),
  ],
  information_schema: [table("COLUMNS", { kind: "view" }), table("TABLES", { kind: "view" })],
  mysql: [
    table("user", {
      engine: "InnoDB",
      rows: 6,
      dataBytes: 16_384,
      indexBytes: 0,
      comment: "Users and global privileges",
    }),
  ],
};

const PG_SCHEMAS: Record<string, string[]> = {
  folio: ["diku_mod_inventory_storage", "diku_mod_users", "public"],
  postgres: ["public"],
};

const PG_TABLES: Record<string, TableInfo[]> = {
  diku_mod_inventory_storage: [
    table("holdings_record", { rows: 1_904_211, dataBytes: 1_288_617_984, indexBytes: 402_653_184 }),
    table("instance", { rows: 2_150_874, dataBytes: 3_221_225_472, indexBytes: 805_306_368, comment: "书目实例" }),
    table("item", { rows: 2_481_002, dataBytes: 1_610_612_736, indexBytes: 536_870_912 }),
    table("instance_holdings_item_view", { kind: "view" }),
    table("hrid_settings_view", { kind: "materializedView", rows: 1, dataBytes: 8_192, indexBytes: 0 }),
  ],
  diku_mod_users: [table("users", { rows: 48_112, dataBytes: 92_274_688, indexBytes: 20_971_520, comment: "读者" })],
  public: [],
};

/** 和 Rust 端的 Endpoint 相等比较一致:缺省的 database 与 null 视为相同。 */
function endpointKey(endpoint: Endpoint): string {
  return endpoint.kind === "file"
    ? `file:${endpoint.path}`
    : `net:${endpoint.host}:${endpoint.port}:${endpoint.username}:${endpoint.database ?? ""}`;
}

function viewOf(config: ConnectionConfig, open: Set<string>): ConnectionView {
  return { ...config, isOpen: open.has(config.id) };
}

function validate(draft: ConnectionDraft): FieldError[] {
  const errors: FieldError[] = [];
  if (!draft.name.trim()) errors.push({ field: "name", message: "请填写连接名" });
  if (draft.endpoint.kind === "network") {
    if (!draft.endpoint.host.trim()) errors.push({ field: "host", message: "请填写主机地址" });
    if (!draft.endpoint.port) errors.push({ field: "port", message: "端口范围是 1–65535" });
    if (!draft.endpoint.username.trim()) errors.push({ field: "username", message: "请填写用户名" });
  } else if (!draft.endpoint.path.startsWith("/")) {
    errors.push({ field: "path", message: "请选择数据库文件" });
  }
  return errors;
}

export function createMockBackend(seed: ConnectionConfig[] = SEED_CONNECTIONS): Backend {
  let connections = [...seed];
  const passwords = new Map<string, string>([["mock-mysql", "root"]]);
  const open = new Set<string>();
  const designs = createMockDesigns();
  const tableData = createMockTables();
  const queries = createMockQueries(() => connections, wait);
  const transfer = createMockTransfer(wait);

  const find = (id: string) =>
    connections.find((c) => c.id === id) ?? fail("connection_not_found", `找不到连接：${id}`);
  const requireOpen = (id: string) => {
    const config = find(id);
    if (!open.has(id)) fail("not_connected", `连接尚未打开：${id}`);
    return config;
  };
  /** 对象树里的表和视图:种子数据叠加建表、改名、删除、复制的结果。 */
  const tablesOf = (id: string, database: string, schema: string | null) => {
    const config = requireOpen(id);
    const base =
      config.driver === "sqlite"
        ? [table("notes"), table("tags"), table("recent_notes", { kind: "view" })]
        : config.driver === "postgres"
          ? (PG_TABLES[schema ?? ""] ?? [])
          : (MYSQL_TABLES[database] ?? []);
    return designs.tables(id, database, schema, base);
  };

  return {
    async listConnections() {
      await wait();
      return connections.map((c) => viewOf(c, open));
    },
    async saveConnection(draft) {
      await wait();
      const errors = validate(draft);
      if (errors.length) fail("invalid_config", "连接配置有误", errors);
      const id = draft.id ?? crypto.randomUUID();
      const config: ConnectionConfig = {
        id,
        name: draft.name.trim(),
        group: draft.group?.trim() || null,
        driver: draft.driver,
        endpoint: draft.endpoint,
        savePassword: draft.savePassword,
      };
      const previous = draft.id ? find(draft.id) : undefined;
      connections = previous ? connections.map((c) => (c.id === id ? config : c)) : [...connections, config];
      const newPassword = draft.savePassword && draft.password != null;
      if (newPassword) passwords.set(id, draft.password!);
      if (!draft.savePassword) passwords.delete(id);
      // 与后端一致:只有影响「怎么连」的改动才断开会话。
      const reconnect =
        previous !== undefined &&
        (previous.driver !== config.driver ||
          endpointKey(previous.endpoint) !== endpointKey(config.endpoint) ||
          newPassword);
      if (reconnect) open.delete(id);
      return config;
    },
    async rememberPassword(id, password) {
      await wait();
      const config = find(id);
      passwords.set(id, password);
      connections = connections.map((c) => (c.id === id ? { ...config, savePassword: true } : c));
    },
    async deleteConnection(id) {
      await wait();
      find(id);
      connections = connections.filter((c) => c.id !== id);
      open.delete(id);
      passwords.delete(id);
      return { passwordLeft: false };
    },
    async duplicateConnection(id) {
      await wait();
      const original = find(id);
      const copy = { ...original, id: crypto.randomUUID(), name: `${original.name} 副本` };
      const index = connections.indexOf(original);
      connections = [...connections.slice(0, index + 1), copy, ...connections.slice(index + 1)];
      return copy;
    },
    async testConnection(draft) {
      await wait(400);
      const errors = validate(draft);
      if (errors.length) fail("invalid_config", "连接配置有误", errors);
      if (draft.endpoint.kind === "network" && draft.endpoint.host === "10.9.9.9") {
        fail("timeout", "连接超时，请检查地址、端口和网络");
      }
      return SERVERS[draft.driver];
    },
    async openConnection(id, password) {
      await wait(300);
      const config = find(id);
      const saved = passwords.get(id);
      if (config.endpoint.kind === "network" && !config.savePassword && !password && !saved) {
        fail("authentication", "用户名或密码不正确：Access denied");
      }
      open.add(id);
      return SERVERS[config.driver];
    },
    async closeConnection(id) {
      await wait(60);
      open.delete(id);
    },
    async listDatabases(id) {
      await wait();
      const config = requireOpen(id);
      if (config.driver === "sqlite") return ["main"];
      if (config.driver === "postgres") return Object.keys(PG_SCHEMAS);
      return Object.keys(MYSQL_TABLES).sort();
    },
    async listSchemas(id, database) {
      await wait();
      const config = requireOpen(id);
      return config.driver === "postgres" ? (PG_SCHEMAS[database] ?? []) : [];
    },
    async listTables(id, database, schema) {
      await wait();
      return tablesOf(id, database, schema ?? null);
    },
    async dropTable(id, ref) {
      await wait(120);
      if (!tablesOf(id, ref.database, ref.schema).some((t) => t.name === ref.table)) {
        fail("table_not_found", `找不到表：${ref.table}`);
      }
      designs.drop(id, ref);
    },
    async tableDetails(id, ref) {
      await wait();
      const config = requireOpen(id);
      const info = tablesOf(id, ref.database, ref.schema).find((t) => t.name === ref.table);
      if (!info) fail("database", `找不到表「${ref.table}」，可能已被删除或没有权限。`);
      return mockTableDetails(config.driver, info, designs.ddl(id, config.driver, ref));
    },
    async copyTable(id, ref, name, withData) {
      await wait(160);
      const config = requireOpen(id);
      const target = name.trim();
      if (!target) fail("invalid_data", "新表名不能为空。");
      const existing = tablesOf(id, ref.database, ref.schema);
      if (existing.some((t) => t.name === target)) fail("database", `Table '${target}' already exists`);
      designs.copy(id, config.driver, ref, target);
      return { table: { ...ref, table: target }, rows: withData ? seedTable(ref.table).rows.length : null };
    },
    async readTablePage(id, table, request) {
      await wait();
      const config = requireOpen(id);
      return tableData.read(id, config.driver, table, request);
    },
    async previewTableChanges(id, table, changes) {
      await wait(80);
      const config = requireOpen(id);
      return tableData.preview(id, config.driver, table, changes);
    },
    async applyTableChanges(id, table, changes, expected) {
      await wait(200);
      const config = requireOpen(id);
      return tableData.apply(id, config.driver, table, changes, expected);
    },
    async readTableDesign(id, table) {
      await wait();
      const config = requireOpen(id);
      return designs.read(id, config.driver, table);
    },
    async previewTableDesign(id, original, design) {
      await wait(80);
      const config = requireOpen(id);
      return designs.preview(id, config.driver, original, design);
    },
    async applyTableDesign(id, original, design, expected) {
      await wait(200);
      const config = requireOpen(id);
      return designs.apply(id, config.driver, original, design, expected);
    },
    async pickSqliteFile() {
      await wait(60);
      return "/Users/me/Documents/new.db";
    },
    async scanImportSource(source) {
      await wait();
      return classifyMockImport(mockImportSources(source), connections);
    },
    async importConnections(source, ids) {
      await wait(300);
      const { configs, filledIds, report } = planMockImport(mockImportSources(source), ids, connections);
      const filled = new Set(filledIds);
      connections = [...connections.map((c) => (filled.has(c.id) ? { ...c, savePassword: true } : c)), ...configs];
      for (const item of report.imported) if (item.passwordSaved) passwords.set(item.id, "mock");
      for (const id of filledIds) passwords.set(id, "mock");
      return report;
    },
    async pickNcxFile() {
      await wait(60);
      return "/Users/me/Downloads/connections.ncx";
    },
    async runStatement(editorId, connectionId, scope, sql) {
      requireOpen(connectionId);
      return queries.runStatement(editorId, connectionId, scope, sql);
    },
    cancelStatement: (editorId) => queries.cancelStatement(editorId),
    closeEditor: (editorId) => queries.closeEditor(editorId),
    async listColumns(id, table) {
      await wait(40);
      requireOpen(id);
      return seedTable(table.table).columns;
    },
    searchHistory: (query, connectionId, limit) => queries.searchHistory(query, connectionId, limit),
    clearHistory: () => queries.clearHistory(),
    listSavedQueries: (connectionId, database, schema) => queries.listSavedQueries(connectionId, database, schema),
    readSavedQuery: (query) => queries.readSavedQuery(query),
    saveQuery: (query, sql, overwrite) => queries.saveQuery(query, sql, overwrite),
    deleteSavedQuery: (query) => queries.deleteSavedQuery(query),
    scanNavicatQueries: () => queries.scanNavicatQueries(),
    importNavicatQueries: (ids) => queries.importNavicatQueries(ids),
    // 浏览器里没有原生菜单和窗口关闭按钮,不会有退出请求。
    setUnsavedWork: () => Promise.resolve(),
    onQuitRequested: () => Promise.resolve(() => undefined),
    quitApp: () => Promise.resolve(),
    ...transfer,
    async exportData(jobId, request) {
      requireOpen(request.connectionId);
      return transfer.exportData(jobId, request);
    },
    async previewImport(connectionId, table, source) {
      requireOpen(connectionId);
      return transfer.previewImport(connectionId, table, source);
    },
    async importData(jobId, request) {
      requireOpen(request.connectionId);
      return transfer.importData(jobId, request);
    },
  };
}
