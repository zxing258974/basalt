import { fileName } from "../../lib/platform";
import type { ConnectionConfig, DriverKind } from "../../lib/types";

export interface DriverMeta {
  kind: DriverKind;
  label: string;
  /** 两个字母的徽标,替代各家商标。 */
  monogram: string;
  color: string;
  defaultPort: number | null;
  /** 连接对话框里「初始数据库」的提示。 */
  databaseHint: string;
}

export const DRIVERS: readonly DriverMeta[] = [
  {
    kind: "mysql",
    label: "MySQL",
    monogram: "My",
    color: "var(--db-mysql)",
    defaultPort: 3306,
    databaseHint: "可选，留空则列出全部库",
  },
  {
    kind: "mariadb",
    label: "MariaDB",
    monogram: "Ma",
    color: "var(--db-mariadb)",
    defaultPort: 3306,
    databaseHint: "可选，留空则列出全部库",
  },
  {
    kind: "postgres",
    label: "PostgreSQL",
    monogram: "Pg",
    color: "var(--db-postgres)",
    defaultPort: 5432,
    databaseHint: "留空时连 postgres 库",
  },
  {
    kind: "sqlite",
    label: "SQLite",
    monogram: "Sq",
    color: "var(--db-sqlite)",
    defaultPort: null,
    databaseHint: "",
  },
];

export function driverMeta(kind: DriverKind): DriverMeta {
  const meta = DRIVERS.find((d) => d.kind === kind);
  if (!meta) throw new Error(`未知的数据库类型：${kind}`);
  return meta;
}

/** 列表里一行连接的副标题:主机:端口 或 文件名。 */
export function endpointSummary(config: Pick<ConnectionConfig, "endpoint">): string {
  const { endpoint } = config;
  if (endpoint.kind === "file") return fileName(endpoint.path);
  return `${endpoint.username}@${endpoint.host}:${endpoint.port}`;
}

/** 打开前是否要先问密码:网络连接且没有保存数据库密码,或者 SSH 用密码登录又没保存。
 *  私钥的口令打开前不问,后端发现私钥有口令才问。 */
export function needsPasswordPrompt(config: ConnectionConfig): boolean {
  if (config.endpoint.kind !== "network") return false;
  return !config.savePassword || needsSshPrompt(config);
}

export function needsSshPrompt(config: ConnectionConfig): boolean {
  return config.ssh?.auth === "password" && !config.ssh.saveSecret;
}

/** 经 SSH 隧道时的说明,如「经 deploy@jump.example.test:22」。 */
export function tunnelSummary(config: Pick<ConnectionConfig, "ssh">): string | null {
  const { ssh } = config;
  return ssh ? `经 ${ssh.username}@${ssh.host}:${ssh.port}` : null;
}
