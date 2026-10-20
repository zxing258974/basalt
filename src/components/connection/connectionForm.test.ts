import { describe, expect, it } from "vitest";
import type { ConnectionConfig } from "../../lib/types";
import { defaultName, emptyForm, formFromConfig, toDraft, withDriver } from "./connectionForm";

describe("emptyForm", () => {
  it("prefills sensible defaults per driver", () => {
    const mysql = emptyForm("mysql");
    expect(mysql).toMatchObject({ host: "127.0.0.1", port: "3306", username: "root", savePassword: true });
    expect(emptyForm("postgres")).toMatchObject({ port: "5432", username: "postgres" });
    expect(emptyForm("sqlite")).toMatchObject({ port: "", username: "", savePassword: false });
  });
});

describe("withDriver", () => {
  it("swaps port and username only while they still hold the old defaults", () => {
    const switched = withDriver(emptyForm("mysql"), "postgres");
    expect(switched).toMatchObject({ driver: "postgres", port: "5432", username: "postgres" });

    const custom = { ...emptyForm("mysql"), port: "13306", username: "app" };
    expect(withDriver(custom, "postgres")).toMatchObject({ port: "13306", username: "app" });
  });

  it("returns a new object", () => {
    const form = emptyForm("mysql");
    expect(withDriver(form, "mariadb")).not.toBe(form);
    expect(form.driver).toBe("mysql");
  });
});

describe("defaultName", () => {
  it("names local servers by product and remote ones by host:port", () => {
    expect(defaultName(emptyForm("mysql"))).toBe("本地 MySQL");
    expect(defaultName({ ...emptyForm("postgres"), host: "10.0.8.21" })).toBe("10.0.8.21:5432");
    expect(defaultName({ ...emptyForm("sqlite"), path: "/Users/me/notes.sqlite3" })).toBe("notes");
  });

  it("falls back to the product name while the host is still blank", () => {
    expect(defaultName({ ...emptyForm("postgres"), host: "  " })).toBe("PostgreSQL");
  });
});

describe("toDraft", () => {
  it("builds a network draft and falls back to the default name", () => {
    const draft = toDraft({ ...emptyForm("mysql"), password: "pw", database: " shop " });
    expect(draft).toEqual({
      id: null,
      name: "本地 MySQL",
      group: null,
      driver: "mysql",
      endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root", database: "shop" },
      savePassword: true,
      password: "pw",
    });
  });

  it("turns an unparsable port into 0 so the backend reports it", () => {
    const draft = toDraft({ ...emptyForm("mysql"), port: "abc" });
    expect(draft.endpoint).toMatchObject({ port: 0 });
  });

  it("builds a file draft for SQLite without password", () => {
    const draft = toDraft({ ...emptyForm("sqlite"), path: "/tmp/a.db", password: "ignored" });
    expect(draft.endpoint).toEqual({ kind: "file", path: "/tmp/a.db" });
    expect(draft.password).toBeNull();
  });

  it("keeps the saved password when editing without touching the field", () => {
    const config: ConnectionConfig = {
      id: "c1",
      name: "生产",
      group: "线上",
      driver: "postgres",
      endpoint: { kind: "network", host: "db", port: 5432, username: "ro", database: null },
      savePassword: true,
    };
    const form = formFromConfig(config);
    expect(form).toMatchObject({ id: "c1", group: "线上", database: "", password: "", hadSavedPassword: true });
    expect(toDraft(form).password).toBeUndefined();
    expect(toDraft({ ...form, password: "", passwordTouched: true }).password).toBe("");
  });

  it("sends the typed password when saving is switched on for a connection that had none", () => {
    const config: ConnectionConfig = {
      id: "c2",
      name: "临时",
      driver: "mysql",
      endpoint: { kind: "network", host: "db", port: 3306, username: "app" },
      savePassword: false,
    };
    const form = { ...formFromConfig(config), savePassword: true };
    expect(form.hadSavedPassword).toBe(false);
    expect(toDraft(form).password).toBe("");
    expect(toDraft({ ...form, password: "pw", passwordTouched: true }).password).toBe("pw");
  });
});
