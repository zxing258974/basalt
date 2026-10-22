// 连接对话框的表单状态与「表单 → 提交内容」的转换。全部是纯函数,便于测试。
import { fileName } from "../../lib/platform";
import type {
  ConnectionConfig,
  ConnectionDraft,
  DriverKind,
  SshAuthMethod,
  SshConfigHost,
  SshTunnel,
  SslMode,
  SslSettings,
} from "../../lib/types";
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
  /** 编辑已有连接时,没动过密码框就沿用密码存储里的密码。 */
  passwordTouched: boolean;
  savePassword: boolean;
  /** 打开表单时密码存储里已经存着密码(原配置 savePassword 为真)。 */
  hadSavedPassword: boolean;
  /** 「SSH」页:关掉时下面的设置保留,只是不用。 */
  sshEnabled: boolean;
  sshHost: string;
  sshPort: string;
  sshUsername: string;
  sshAuth: SshAuthMethod;
  sshKeyPath: string;
  /** 跳板机(ProxyJump 写法),空着就是直连 SSH 服务器。 */
  sshJumpHosts: string;
  /** SSH 密码或私钥口令,规则同 password。 */
  sshSecret: string;
  sshSecretTouched: boolean;
  sshSaveSecret: boolean;
  hadSavedSshSecret: boolean;
  /** 「SSL」页。 */
  sslMode: SslMode;
  sslCa: string;
  sslCert: string;
  sslKey: string;
}

/** 对话框的三页。SQLite 只有「常规」。 */
export type FormPage = "general" | "ssh" | "ssl";

export const DEFAULT_SSH_PORT = "22";

/** SSL 模式的名字,导入预览、SSL 页共用。 */
export const SSL_MODE_LABELS: Readonly<Record<SslMode, string>> = {
  disabled: "禁用",
  preferred: "优先",
  required: "必须",
  verifyCa: "校验 CA",
  verifyIdentity: "校验主机名",
};

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
    sshEnabled: false,
    sshHost: "",
    sshPort: DEFAULT_SSH_PORT,
    sshUsername: "",
    sshAuth: "password",
    sshKeyPath: "",
    sshJumpHosts: "",
    sshSecret: "",
    sshSecretTouched: false,
    sshSaveSecret: true,
    hadSavedSshSecret: false,
    sslMode: "preferred",
    sslCa: "",
    sslCert: "",
    sslKey: "",
  };
}

function sshFields(ssh: SshTunnel | null | undefined): Partial<ConnectionForm> {
  if (!ssh) return {};
  return {
    sshEnabled: true,
    sshHost: ssh.host,
    sshPort: String(ssh.port),
    sshUsername: ssh.username,
    sshAuth: ssh.auth,
    sshKeyPath: ssh.privateKeyPath ?? "",
    sshJumpHosts: ssh.jumpHosts ?? "",
    sshSaveSecret: ssh.saveSecret,
    hadSavedSshSecret: ssh.saveSecret,
  };
}

function sslFields(ssl: SslSettings | undefined): Partial<ConnectionForm> {
  if (!ssl) return {};
  return {
    sslMode: ssl.mode,
    sslCa: ssl.caPath ?? "",
    sslCert: ssl.clientCertPath ?? "",
    sslKey: ssl.clientKeyPath ?? "",
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
    ...sshFields(config.ssh),
    ...sslFields(config.ssl),
  };
}

/** 换登录方式:原来存的是密码还是口令说不准,框里清空,当作重新填过。 */
export function withSshAuth(form: ConnectionForm, sshAuth: SshAuthMethod): ConnectionForm {
  if (sshAuth === form.sshAuth) return form;
  return { ...form, sshAuth, sshSecret: "", sshSecretTouched: true };
}

/**
 * 按 ~/.ssh/config 里的一台主机填 SSH 页:地址、端口、用户名、跳板机;配了私钥就用私钥登录,没配用 ssh-agent。
 * 用户名没写的保留原来填的。
 */
export function withSshConfigHost(form: ConnectionForm, host: SshConfigHost): ConnectionForm {
  const auth: SshAuthMethod = host.privateKeyPath ? "privateKey" : "agent";
  return {
    ...withSshAuth(form, auth),
    sshEnabled: true,
    sshHost: host.host,
    sshPort: String(host.port),
    sshUsername: host.username ?? form.sshUsername,
    sshKeyPath: host.privateKeyPath ?? "",
    sshJumpHosts: host.jumpHosts ?? "",
  };
}

/** 这种登录方式要不要密码框:ssh-agent 不要。 */
export const sshUsesSecret = (auth: SshAuthMethod) => auth !== "agent";

/** SSH 那个密码框叫什么:密码登录是 SSH 密码,私钥登录是私钥口令。 */
export function sshSecretLabel(auth: SshAuthMethod): string {
  return auth === "password" ? "SSH 密码" : "私钥口令";
}

/** 字段错误在哪一页:保存失败时切到第一个出错的页。 */
export function pageOfField(field: string): FormPage {
  if (field.startsWith("ssh")) return "ssh";
  if (field.startsWith("ssl")) return "ssl";
  return "general";
}

/** 这一页的设置是不是和默认不一样:页签上点一个小圆点。 */
export function pageIsActive(form: ConnectionForm, page: FormPage): boolean {
  if (page === "ssh") return form.sshEnabled;
  if (page === "ssl") return form.sslMode !== "preferred";
  return false;
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
    const file = fileName(form.path);
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
  // 只有密码存储里本来就有密码时,「没动密码框」才意味着沿用;否则就用框里的内容(可以是空密码)。
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
    ...sshDraft(form),
    ssl: {
      mode: form.sslMode,
      caPath: form.sslCa.trim() || null,
      clientCertPath: form.sslCert.trim() || null,
      clientKeyPath: form.sslKey.trim() || null,
    },
  };
}

function sshDraft(form: ConnectionForm): Pick<ConnectionDraft, "ssh" | "sshSecret"> {
  if (!form.sshEnabled) return { ssh: null, sshSecret: null };
  const keepSavedSecret = form.hadSavedSshSecret && !form.sshSecretTouched;
  return {
    ssh: {
      host: form.sshHost.trim(),
      port: parsePort(form.sshPort),
      username: form.sshUsername.trim(),
      auth: form.sshAuth,
      privateKeyPath: form.sshAuth === "privateKey" ? form.sshKeyPath.trim() || null : null,
      // ssh-agent 登录没有要存的东西。
      saveSecret: sshUsesSecret(form.sshAuth) && form.sshSaveSecret,
      jumpHosts: form.sshJumpHosts.trim() || null,
    },
    // 私钥没填口令就是没有口令;密码登录没填密码时不传,测试连接会说缺 SSH 密码。ssh-agent 不用密码。
    sshSecret:
      form.sshAuth === "agent"
        ? null
        : keepSavedSecret
          ? undefined
          : form.sshAuth === "password" && !form.sshSecret
            ? null
            : form.sshSecret,
  };
}
