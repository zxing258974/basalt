// 连接对话框的表单状态与「表单 → 提交内容」的转换。全部是纯函数,便于测试。
import type { ConnectionConfig, ConnectionDraft, DriverKind } from "../../lib/types";
import { driverMeta } from "./driverMeta";

export interface ConnectionForm {
  /** null 表示新建。 */
  id: string | null;
  name: string;
  group: string;
  driver: DriverKind;
  host: string;
  /** 保留输入原文,提交时再解析,便于显示后端的端口校验错误。 */
  port: string;
  username: string;
  database: string;
  path: string;
  password: string;
  /** 编辑已有连接时,没动过密码框就沿用钥匙串里的密码。 */
  passwordTouched: boolean;
  savePassword: boolean;
  /** 打开表单时钥匙串里已经存着密码(原配置 savePassword 为真)。 */
  hadSavedPassword: boolean;
}

const LOCAL_HOSTS = new Set(["127.0.0.1", "localhost", "::1"]);

function defaultUsername(driver: DriverKind): string {
  switch (driver) {
    case "mysql":
    case "mariadb":
      return "root";
    case "postgres":
      return "postgres";
    case "sqlite":
      return "";
  }
}

const defaultPort = (driver: DriverKind) => String(driverMeta(driver).defaultPort ?? "");

export function emptyForm(driver: DriverKind): ConnectionForm {
  const isFile = driver === "sqlite";
  return {
    id: null,
    name: "",
    group: "",
    driver,
    host: isFile ? "" : "127.0.0.1",
    port: defaultPort(driver),
    username: defaultUsername(driver),
    database: "",
    path: "",
    password: "",
    passwordTouched: false,
    savePassword: !isFile,
    hadSavedPassword: false,
  };
}

export function formFromConfig(config: ConnectionConfig): ConnectionForm {
  const base = { ...emptyForm(config.driver), id: config.id, name: config.name, group: config.group ?? "" };
  if (config.endpoint.kind === "file") {
    return { ...base, path: config.endpoint.path, savePassword: config.savePassword };
  }
  return {
    ...base,
    host: config.endpoint.host,
    port: String(config.endpoint.port),
    username: config.endpoint.username,
    database: config.endpoint.database ?? "",
    savePassword: config.savePassword,
    hadSavedPassword: config.savePassword,
  };
}

/** 换数据库类型时,只有仍是旧默认值的端口和用户名才跟着换。 */
export function withDriver(form: ConnectionForm, driver: DriverKind): ConnectionForm {
  const fresh = emptyForm(driver);
  return {
    ...form,
    driver,
    port: form.port === defaultPort(form.driver) ? fresh.port : form.port,
    username: form.username === defaultUsername(form.driver) ? fresh.username : form.username,
    host: form.host || fresh.host,
  };
}

export function defaultName(form: ConnectionForm): string {
  if (form.driver === "sqlite") {
    const file = form.path.split("/").pop() ?? "";
    return file.replace(/\.[^.]+$/, "") || "SQLite";
  }
  const host = form.host.trim();
  const { label } = driverMeta(form.driver);
  if (!host) return label;
  if (LOCAL_HOSTS.has(host)) return `本地 ${label}`;
  return `${host}:${form.port.trim()}`;
}

function parsePort(raw: string): number {
  const port = Number(raw.trim());
  return Number.isInteger(port) && port > 0 && port <= 65_535 ? port : 0;
}

export function toDraft(form: ConnectionForm): ConnectionDraft {
  const base = {
    id: form.id,
    name: form.name.trim() || defaultName(form),
    group: form.group.trim() || null,
    driver: form.driver,
  };
  if (form.driver === "sqlite") {
    return {
      ...base,
      endpoint: { kind: "file", path: form.path.trim() },
      savePassword: false,
      password: null,
    };
  }
  // 只有钥匙串里本来就有密码时,「没动密码框」才意味着沿用;否则就用框里的内容(可以是空密码)。
  const keepSavedPassword = form.hadSavedPassword && !form.passwordTouched;
  return {
    ...base,
    endpoint: {
      kind: "network",
      host: form.host.trim(),
      port: parsePort(form.port),
      username: form.username.trim(),
      database: form.database.trim() || null,
    },
    savePassword: form.savePassword,
    password: keepSavedPassword ? undefined : form.password,
  };
}
