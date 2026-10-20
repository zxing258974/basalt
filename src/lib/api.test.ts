// 前端与 Rust 命令的对接约定:命令名、参数名,以及缺省参数要传 null 而不是不传。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getBackend, isTauriRuntime, setBackendForTesting, type Backend } from "./api";
import type { ConnectionDraft } from "./types";

const invoke = vi.hoisted(() => vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>());
const openDialog = vi.hoisted(() => vi.fn<(options: unknown) => Promise<string | string[] | null>>());
const listen = vi.hoisted(() => vi.fn<(event: string, handler: () => void) => Promise<() => void>>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openDialog }));

const tauriWindow = window as unknown as Record<string, unknown>;
const scope = { database: "shop", schema: null };
const table = { database: "shop", schema: null, table: "orders" };
const saved = { connectionId: "c1", database: "shop", schema: null, name: "日报" };

const draft: ConnectionDraft = {
  id: null,
  name: "本地",
  group: null,
  driver: "mysql",
  endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root" },
  savePassword: false,
  password: null,
};

describe("getBackend in a plain browser", () => {
  it("falls back to the mock backend and keeps using the same instance", async () => {
    expect(isTauriRuntime()).toBe(false);
    const first = await getBackend();
    expect(await getBackend()).toBe(first);
    expect((await first.listConnections()).length).toBeGreaterThan(0);
  });

  it("uses the backend injected for tests", async () => {
    const fake = { listConnections: vi.fn() } as unknown as Backend;
    setBackendForTesting(fake);
    expect(await getBackend()).toBe(fake);
  });
});

describe("Tauri backend", () => {
  beforeEach(() => {
    tauriWindow.__TAURI_INTERNALS__ = {};
    invoke.mockReset().mockResolvedValue(undefined);
    openDialog.mockReset();
  });
  afterEach(() => {
    delete tauriWindow.__TAURI_INTERNALS__;
  });

  type Call = [name: string, run: (backend: Backend) => Promise<unknown>, command: string, args?: unknown];
  const calls: Call[] = [
    ["listConnections", (b) => b.listConnections(), "list_connections"],
    ["saveConnection", (b) => b.saveConnection(draft), "save_connection", { draft }],
    ["rememberPassword", (b) => b.rememberPassword("c1", "pw"), "remember_password", { id: "c1", password: "pw" }],
    ["deleteConnection", (b) => b.deleteConnection("c1"), "delete_connection", { id: "c1" }],
    ["duplicateConnection", (b) => b.duplicateConnection("c1"), "duplicate_connection", { id: "c1" }],
    ["testConnection", (b) => b.testConnection(draft), "test_connection", { draft }],
    ["openConnection without password", (b) => b.openConnection("c1"), "open_connection", { id: "c1", password: null }],
    [
      "openConnection with password",
      (b) => b.openConnection("c1", "pw"),
      "open_connection",
      { id: "c1", password: "pw" },
    ],
    ["closeConnection", (b) => b.closeConnection("c1"), "close_connection", { id: "c1" }],
    ["listDatabases", (b) => b.listDatabases("c1"), "list_databases", { id: "c1" }],
    ["listSchemas", (b) => b.listSchemas("c1", "folio"), "list_schemas", { id: "c1", database: "folio" }],
    [
      "listTables without schema",
      (b) => b.listTables("c1", "shop"),
      "list_tables",
      { id: "c1", database: "shop", schema: null },
    ],
    [
      "listTables with schema",
      (b) => b.listTables("c1", "folio", "public"),
      "list_tables",
      { id: "c1", database: "folio", schema: "public" },
    ],
    [
      "runStatement",
      (b) => b.runStatement("tab", "c1", scope, "SELECT 1"),
      "run_statement",
      { editorId: "tab", connectionId: "c1", scope, sql: "SELECT 1" },
    ],
    ["cancelStatement", (b) => b.cancelStatement("tab"), "cancel_statement", { editorId: "tab" }],
    ["closeEditor", (b) => b.closeEditor("tab"), "close_editor", { editorId: "tab" }],
    ["listColumns", (b) => b.listColumns("c1", table), "list_columns", { id: "c1", table }],
    [
      "searchHistory",
      (b) => b.searchHistory("orders", null, 200),
      "search_history",
      { query: "orders", connectionId: null, limit: 200 },
    ],
    ["clearHistory", (b) => b.clearHistory(), "clear_history"],
    [
      "listSavedQueries",
      (b) => b.listSavedQueries("c1", "folio", "public"),
      "list_saved_queries",
      { connectionId: "c1", database: "folio", schema: "public" },
    ],
    ["readSavedQuery", (b) => b.readSavedQuery(saved), "read_saved_query", { query: saved }],
    [
      "saveQuery",
      (b) => b.saveQuery(saved, "SELECT 1", true),
      "save_query",
      { query: saved, sql: "SELECT 1", overwrite: true },
    ],
    ["deleteSavedQuery", (b) => b.deleteSavedQuery(saved), "delete_saved_query", { query: saved }],
    ["scanNavicatQueries", (b) => b.scanNavicatQueries(), "scan_navicat_queries"],
    ["importNavicatQueries", (b) => b.importNavicatQueries(["a"]), "import_navicat_queries", { ids: ["a"] }],
    ["setUnsavedWork", (b) => b.setUnsavedWork(true), "set_unsaved_work", { unsaved: true }],
    ["quitApp", (b) => b.quitApp(), "quit_app"],
  ];

  it.each(calls)("%s invokes the matching command", async (_name, run, command, args) => {
    await run(await getBackend());
    expect(invoke).toHaveBeenCalledWith(command, args);
  });

  it("passes the command result through", async () => {
    invoke.mockResolvedValue(["shop", "mysql"]);
    expect(await (await getBackend()).listDatabases("c1")).toEqual(["shop", "mysql"]);
  });

  it("listens for the quit request the Rust side sends", async () => {
    const unlisten = vi.fn();
    let emit = () => {};
    listen.mockImplementationOnce((_event, onEvent) => {
      emit = onEvent;
      return Promise.resolve(unlisten);
    });
    const handler = vi.fn();
    expect(await (await getBackend()).onQuitRequested(handler)).toBe(unlisten);
    expect(listen).toHaveBeenCalledWith("quit-requested", expect.any(Function));
    emit();
    expect(handler).toHaveBeenCalledOnce();
  });

  it("asks the native dialog for a single SQLite file", async () => {
    const backend = await getBackend();
    openDialog.mockResolvedValueOnce("/Users/me/a.db");
    expect(await backend.pickSqliteFile()).toBe("/Users/me/a.db");
    expect(openDialog).toHaveBeenCalledWith(expect.objectContaining({ multiple: false, directory: false }));

    openDialog.mockResolvedValueOnce(null);
    expect(await backend.pickSqliteFile()).toBeNull();
  });
});
