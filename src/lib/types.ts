// 与 crates/basalt-core 的序列化结构一一对应。

export type DriverKind = "mysql" | "mariadb" | "postgres" | "sqlite";

export type Endpoint =
  | { kind: "network"; host: string; port: number; username: string; database?: string | null }
  | { kind: "file"; path: string };

export type SshAuthMethod = "password" | "privateKey";

/** 经 SSH 服务器转发;数据库的主机、端口是从 SSH 服务器那边看的地址。密码不在这里。 */
export interface SshTunnel {
  host: string;
  port: number;
  username: string;
  auth: SshAuthMethod;
  privateKeyPath?: string | null;
  /** SSH 密码(或私钥口令)存在钥匙串里。 */
  saveSecret: boolean;
}

export type SslMode = "disabled" | "preferred" | "required" | "verifyCa" | "verifyIdentity";

export interface SslSettings {
  mode: SslMode;
  caPath?: string | null;
  clientCertPath?: string | null;
  clientKeyPath?: string | null;
}

/** 第一次连接的 SSH 服务器出示的主机密钥,交给用户核对指纹。 */
export interface HostKey {
  host: string;
  port: number;
  algorithm: string;
  /** SHA256: 开头。 */
  fingerprint: string;
  publicKey: string;
}

export interface ConnectionConfig {
  id: string;
  name: string;
  group?: string | null;
  driver: DriverKind;
  endpoint: Endpoint;
  savePassword: boolean;
  /** M4 以前保存的连接没有这两项:当作直连、SSL 优先。 */
  ssh?: SshTunnel | null;
  ssl?: SslSettings;
}

/** 信息面板里一个属性的值;数字交给前端排版。 */
export type PropertyValue =
  | { kind: "text"; value: string }
  | { kind: "count"; value: number }
  | { kind: "bytes"; value: number }
  | { kind: "dateTime"; value: string };

export interface TableProperty {
  label: string;
  /** null:数据库没给,显示「—」。 */
  value: PropertyValue | null;
}

/** 信息面板:属性和建表语句。ddlGenerated:按表结构生成的(PostgreSQL),不是数据库给的原文。 */
export interface TableDetails {
  properties: TableProperty[];
  ddl: string;
  ddlGenerated: boolean;
}

/** 复制表的结果。rows:复制了多少行;只复制结构时为 null。 */
export interface CopyTableOutcome {
  table: TableRef;
  rows: number | null;
}

/** 删除连接的结果。passwordLeft:密码要授权才能从钥匙串删(旧版 Basalt 存的),或钥匙串出错,还留在钥匙串里。 */
export interface DeletedConnection {
  passwordLeft: boolean;
}

export interface ConnectionView extends ConnectionConfig {
  isOpen: boolean;
}

export interface ConnectionDraft {
  id?: string | null;
  name: string;
  group?: string | null;
  driver: DriverKind;
  endpoint: Endpoint;
  savePassword: boolean;
  /** 不传表示沿用已保存的密码。 */
  password?: string | null;
  ssh?: SshTunnel | null;
  ssl?: SslSettings;
  /** SSH 密码或私钥口令,规则同 password。 */
  sshSecret?: string | null;
}

export interface ServerInfo {
  product: string;
  version: string;
}

export type TableKind = "table" | "view" | "materializedView" | "foreignTable";

export interface TableInfo {
  name: string;
  kind: TableKind;
  engine: string | null;
  rows: number | null;
  dataBytes: number | null;
  indexBytes: number | null;
  createdAt: string | null;
  updatedAt: string | null;
  comment: string | null;
}

/** 单元格值,对应 core 的 CellValue。整数、浮点都是字符串,不丢精度。 */
export type CellValue =
  | { kind: "null" }
  | { kind: "bool"; value: boolean }
  | {
      kind: "int" | "uint" | "float" | "decimal" | "text" | "bytes" | "json" | "date" | "time" | "dateTime";
      value: string;
    }
  | { kind: "unsupported"; value: { typeName: string } };

/** 写回时怎么解释输入;二进制列才把 0x 开头的输入当十六进制。 */
export type ValueType =
  { kind: "integer"; unsigned: boolean } | { kind: "float" | "decimal" | "bit" | "binary" | "text" };

export interface ColumnInfo {
  name: string;
  /** 给人看的完整类型,如 varchar(50)。 */
  dataType: string;
  isPrimaryKey: boolean;
  nullable: boolean;
  /** 有默认值或自增,新增行时可以不填。 */
  hasDefault: boolean;
  isGenerated: boolean;
  valueType: ValueType;
}

export interface TableRef {
  database: string;
  schema: string | null;
  table: string;
}

export interface SortSpec {
  column: string;
  descending: boolean;
}

export interface PageRequest {
  offset: number;
  limit: number;
  sort: SortSpec | null;
  /** WHERE 后面的条件原文。 */
  filter: string | null;
}

export interface TablePage {
  columns: ColumnInfo[];
  rows: CellValue[][];
  hasMore: boolean;
  /** 不能编辑时的原因。 */
  readOnlyReason: string | null;
  /** 实际执行的 SELECT。 */
  sql: string;
}

/** 编辑框里的新值:null 是 SQL NULL。 */
export type EditInput = string | null;

export type RowChange =
  | { kind: "update"; key: Record<string, CellValue>; values: Record<string, EditInput> }
  | { kind: "insert"; values: Record<string, EditInput> }
  | { kind: "delete"; key: Record<string, CellValue> };

export interface ApplyOutcome {
  statements: number;
  affectedRows: number;
}

export interface FieldError {
  field: string;
  message: string;
}

export interface CommandError {
  code: string;
  message: string;
  fields: FieldError[];
  /** code 为 host_key_unknown 时:要用户核对的 SSH 主机密钥。 */
  hostKey?: HostKey;
}

export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as CommandError).code === "string" &&
    typeof (value as CommandError).message === "string"
  );
}

export function errorMessage(error: unknown): string {
  if (isCommandError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return String(error);
}
