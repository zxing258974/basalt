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
  if (endpoint.kind === "file") return endpoint.path.split("/").pop() ?? endpoint.path;
  return `${endpoint.username}@${endpoint.host}:${endpoint.port}`;
}

/** 打开前是否要先问密码:网络连接且没有保存密码。 */
export function needsPasswordPrompt(config: ConnectionConfig): boolean {
  return config.endpoint.kind === "network" && !config.savePassword;
}
