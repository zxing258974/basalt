import { describe, expect, it } from "vitest";
import type { ConnectionConfig } from "../../lib/types";
import {
  defaultName,
  emptyForm,
  formFromConfig,
  toDraft,
  withDriver,
  pageIsActive,
  pageOfField,
  sshSecretLabel,
  sshUsesSecret,
  withSshAuth,
  withSshConfigHost,
} from "./connectionForm";

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
    expect(defaultName({ ...emptyForm("sqlite"), path: "C:\\Users\\me\\账本.db" })).toBe("账本");
    expect(defaultName(emptyForm("sqlite"))).toBe("SQLite");
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
      ssh: null,
      sshSecret: null,
      ssl: { mode: "preferred", caPath: null, clientCertPath: null, clientKeyPath: null },
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

describe("tunnel and SSL", () => {
  const tunneled: ConnectionConfig = {
    id: "t1",
    name: "跳板机后的库",
    driver: "postgres",
    endpoint: { kind: "network", host: "127.0.0.1", port: 5432, username: "app", database: null },
    savePassword: true,
    ssh: {
      host: "jump",
      port: 2222,
      username: "deploy",
      auth: "privateKey",
      privateKeyPath: "/k/id",
      saveSecret: true,
      jumpHosts: "ops@bastion:2200",
    },
    ssl: { mode: "verifyCa", caPath: "/c/ca.pem" },
  };

  it("round-trips a saved tunnel and keeps its saved secret untouched", () => {
    const form = formFromConfig(tunneled);
    expect(form).toMatchObject({
      sshEnabled: true,
      sshPort: "2222",
      sshAuth: "privateKey",
      sshJumpHosts: "ops@bastion:2200",
      hadSavedSshSecret: true,
      sslMode: "verifyCa",
      sslCa: "/c/ca.pem",
      sslCert: "",
    });
    const draft = toDraft(form);
    expect(draft.ssh).toEqual(tunneled.ssh);
    expect(draft.sshSecret).toBeUndefined();
    expect(draft.ssl).toEqual({ mode: "verifyCa", caPath: "/c/ca.pem", clientCertPath: null, clientKeyPath: null });
  });

  it("sends what was typed, an empty passphrase, or nothing for an empty SSH password", () => {
    const form = formFromConfig(tunneled);
    expect(toDraft({ ...form, sshSecret: "kp", sshSecretTouched: true }).sshSecret).toBe("kp");
    expect(toDraft({ ...form, sshSecret: "", sshSecretTouched: true }).sshSecret).toBe("");
    const password = withSshAuth(form, "password");
    expect(password).toMatchObject({ sshSecret: "", sshSecretTouched: true });
    expect(toDraft(password).sshSecret).toBeNull();
    expect(toDraft(password).ssh?.privateKeyPath).toBeNull();
    expect(withSshAuth(password, "password")).toBe(password);
  });

  it("drops the tunnel when it is switched off", () => {
    const off = { ...formFromConfig(tunneled), sshEnabled: false };
    expect(toDraft(off)).toMatchObject({ ssh: null, sshSecret: null });
    expect(pageIsActive(off, "ssh")).toBe(false);
    expect(pageIsActive(off, "ssl")).toBe(true);
    expect(pageIsActive(off, "general")).toBe(false);
  });

  it("logs in through ssh-agent without a secret and keeps jump hosts", () => {
    const agent = { ...withSshAuth(formFromConfig(tunneled), "agent"), sshSecret: "ignored", sshJumpHosts: "  a, b  " };
    const draft = toDraft(agent);
    expect(draft.ssh).toMatchObject({ auth: "agent", privateKeyPath: null, saveSecret: false, jumpHosts: "a, b" });
    expect(draft.sshSecret).toBeNull();
    expect(sshUsesSecret("agent")).toBe(false);
    expect(toDraft({ ...agent, sshJumpHosts: " " }).ssh?.jumpHosts).toBeNull();
  });

  it("fills the SSH page from a ~/.ssh/config host", () => {
    const form = { ...emptyForm("postgres"), sshUsername: "me" };
    const withKey = withSshConfigHost(form, {
      alias: "bastion",
      host: "bastion.example.com",
      port: 2200,
      username: "ops",
      privateKeyPath: "/k/ops",
      jumpHosts: null,
    });
    expect(withKey).toMatchObject({
      sshEnabled: true,
      sshHost: "bastion.example.com",
      sshPort: "2200",
      sshUsername: "ops",
      sshAuth: "privateKey",
      sshKeyPath: "/k/ops",
      sshJumpHosts: "",
    });
    const viaAgent = withSshConfigHost(form, {
      alias: "gw",
      host: "10.0.3.7",
      port: 22,
      username: null,
      privateKeyPath: null,
      jumpHosts: "bastion",
    });
    expect(viaAgent).toMatchObject({ sshAuth: "agent", sshUsername: "me", sshJumpHosts: "bastion", sshKeyPath: "" });
  });

  it("knows which page a field error belongs to", () => {
    expect(pageOfField("sshHost")).toBe("ssh");
    expect(pageOfField("sslKey")).toBe("ssl");
    expect(pageOfField("host")).toBe("general");
    expect(sshSecretLabel("password")).toBe("SSH 密码");
    expect(sshSecretLabel("privateKey")).toBe("私钥口令");
  });
});
