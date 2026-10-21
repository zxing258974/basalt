// mock 后端是浏览器里调界面和截图测试的数据来源,行为要和 Rust 端对得上。
import { describe, expect, it } from "vitest";
import { createMockBackend } from "./mockBackend";
import type { ConnectionConfig, ConnectionDraft } from "./types";

const localEndpoint = { kind: "network" as const, host: "127.0.0.1", port: 3306, username: "root" };

const mysql: ConnectionConfig = {
  id: "my",
  name: "本地",
  driver: "mysql",
  endpoint: localEndpoint,
  savePassword: true,
};

const pg: ConnectionConfig = {
  id: "pg",
  name: "远程",
  driver: "postgres",
  endpoint: { kind: "network", host: "10.0.0.5", port: 5432, username: "app", database: "folio" },
  savePassword: false,
};

const sqlite: ConnectionConfig = {
  id: "lite",
  name: "笔记",
  driver: "sqlite",
  endpoint: { kind: "file", path: "/tmp/notes.db" },
  savePassword: false,
};

const draftOf = (config: ConnectionConfig, patch: Partial<ConnectionDraft> = {}): ConnectionDraft => ({
  ...config,
  group: config.group ?? null,
  password: null,
  ...patch,
});

describe("connections", () => {
  it("starts from the seed with nothing open", async () => {
    const backend = createMockBackend();
    const connections = await backend.listConnections();
    expect(connections.map((c) => c.driver)).toEqual(["mysql", "postgres", "sqlite"]);
    expect(connections.every((c) => !c.isOpen)).toBe(true);
  });

  it("reports every missing network field at once", async () => {
    const backend = createMockBackend([]);
    const draft = draftOf(mysql, {
      id: null,
      name: " ",
      endpoint: { kind: "network", host: "", port: 0, username: "" },
    });
    await expect(backend.saveConnection(draft)).rejects.toMatchObject({
      code: "invalid_config",
      fields: [{ field: "name" }, { field: "host" }, { field: "port" }, { field: "username" }],
    });
  });

  it("requires an absolute path for SQLite", async () => {
    const backend = createMockBackend([]);
    const draft = draftOf(sqlite, { id: null, endpoint: { kind: "file", path: "notes.db" } });
    await expect(backend.testConnection(draft)).rejects.toMatchObject({ fields: [{ field: "path" }] });
  });

  it("adds a new connection with a trimmed name and no empty group", async () => {
    const backend = createMockBackend([mysql]);
    const saved = await backend.saveConnection(draftOf(sqlite, { id: null, name: "  新库 ", group: "  " }));
    expect(saved).toMatchObject({ name: "新库", group: null });
    expect((await backend.listConnections()).map((c) => c.id)).toEqual(["my", saved.id]);
  });

  it("keeps the session when only the name changes", async () => {
    const backend = createMockBackend([mysql]);
    await backend.openConnection("my");
    await backend.saveConnection(draftOf(mysql, { name: "改名" }));
    expect((await backend.listConnections())[0]).toMatchObject({ name: "改名", isOpen: true });
  });

  it.each([
    ["the host changes", { endpoint: { ...localEndpoint, host: "10.0.0.9" } }],
    ["the driver changes", { driver: "mariadb" as const }],
    ["a new password is saved", { password: "secret" }],
  ])("drops the session when %s", async (_case, patch) => {
    const backend = createMockBackend([mysql]);
    await backend.openConnection("my");
    await backend.saveConnection(draftOf(mysql, patch));
    expect((await backend.listConnections())[0]?.isOpen).toBe(false);
  });

  it("forgets the saved password when saving is turned off", async () => {
    const backend = createMockBackend([{ ...pg, savePassword: true }]);
    await backend.saveConnection(draftOf(pg, { savePassword: true, password: "pw" }));
    await backend.saveConnection(draftOf(pg, { savePassword: false }));
    await expect(backend.openConnection("pg")).rejects.toMatchObject({ code: "authentication" });
  });

  it("remembers a password and marks the connection as saving it", async () => {
    const backend = createMockBackend([pg]);
    await backend.rememberPassword("pg", "pw");
    expect((await backend.listConnections())[0]?.savePassword).toBe(true);
    await expect(backend.openConnection("pg")).resolves.toMatchObject({ product: "PostgreSQL" });
  });

  it("puts a duplicate right after the original", async () => {
    const backend = createMockBackend([mysql, sqlite]);
    const copy = await backend.duplicateConnection("my");
    expect(copy.name).toBe("本地 副本");
    expect((await backend.listConnections()).map((c) => c.id)).toEqual(["my", copy.id, "lite"]);
  });

  it("deletes a connection and rejects unknown ids", async () => {
    const backend = createMockBackend([mysql]);
    await backend.openConnection("my");
    expect(await backend.deleteConnection("my")).toEqual({ passwordLeft: false });
    expect(await backend.listConnections()).toEqual([]);
    await expect(backend.deleteConnection("my")).rejects.toMatchObject({ code: "connection_not_found" });
  });
});

describe("sessions", () => {
  it("returns the server version on a successful test", async () => {
    const backend = createMockBackend([]);
    await expect(backend.testConnection(draftOf(sqlite))).resolves.toEqual({ product: "SQLite", version: "3.46.0" });
  });

  it("times out on the unreachable test host", async () => {
    const backend = createMockBackend([]);
    const draft = draftOf(mysql, { endpoint: { ...localEndpoint, host: "10.9.9.9" } });
    await expect(backend.testConnection(draft)).rejects.toMatchObject({ code: "timeout" });
  });

  it("needs a password for a network connection without a saved one", async () => {
    const backend = createMockBackend([pg]);
    await expect(backend.openConnection("pg")).rejects.toMatchObject({ code: "authentication" });
    await backend.openConnection("pg", "pw");
    expect((await backend.listConnections())[0]?.isOpen).toBe(true);
    await backend.closeConnection("pg");
    expect((await backend.listConnections())[0]?.isOpen).toBe(false);
  });

  it("refuses metadata calls before the connection is open", async () => {
    const backend = createMockBackend([mysql]);
    await expect(backend.listDatabases("my")).rejects.toMatchObject({ code: "not_connected" });
  });
});

describe("metadata", () => {
  async function opened(config: ConnectionConfig) {
    const backend = createMockBackend([config]);
    await backend.openConnection(config.id, "pw");
    return backend;
  }

  it("lists MySQL databases in name order with their tables", async () => {
    const backend = await opened(mysql);
    expect(await backend.listDatabases("my")).toEqual(["information_schema", "mysql", "shop"]);
    expect(await backend.listSchemas("my", "shop")).toEqual([]);
    expect((await backend.listTables("my", "shop")).map((t) => t.name)).toContain("orders");
    expect(await backend.listTables("my", "missing")).toEqual([]);
  });

  it("lists PostgreSQL schemas and the tables inside one", async () => {
    const backend = await opened(pg);
    expect(await backend.listDatabases("pg")).toEqual(["folio", "postgres"]);
    expect(await backend.listSchemas("pg", "folio")).toContain("diku_mod_users");
    expect(await backend.listSchemas("pg", "missing")).toEqual([]);
    expect((await backend.listTables("pg", "folio", "diku_mod_users")).map((t) => t.name)).toEqual(["users"]);
    expect(await backend.listTables("pg", "folio", null)).toEqual([]);
  });

  it("shows SQLite as a single main database", async () => {
    const backend = await opened(sqlite);
    expect(await backend.listDatabases("lite")).toEqual(["main"]);
    expect((await backend.listTables("lite", "main")).map((t) => t.kind)).toEqual(["table", "table", "view"]);
  });

  it("picks a fixed SQLite file", async () => {
    await expect(createMockBackend([]).pickSqliteFile()).resolves.toMatch(/\.db$/);
  });
});
