import type {
  ConnectionConfig,
  ConnectionDraft,
  ConnectionView,
  ServerInfo,
  TableInfo,
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
  pickSqliteFile(): Promise<string | null>;
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
    listTables: (id, database, schema) =>
      call("list_tables", { id, database, schema: schema ?? null }),
    pickSqliteFile: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "SQLite", extensions: ["db", "sqlite", "sqlite3", "db3"] }],
      });
      return typeof picked === "string" ? picked : null;
    },
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
