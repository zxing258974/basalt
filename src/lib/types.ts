// 与 crates/basalt-core 的序列化结构一一对应。

export type DriverKind = "mysql" | "mariadb" | "postgres" | "sqlite";

export type Endpoint =
  | { kind: "network"; host: string; port: number; username: string; database?: string | null }
  | { kind: "file"; path: string };

export interface ConnectionConfig {
  id: string;
  name: string;
  group?: string | null;
  driver: DriverKind;
  endpoint: Endpoint;
  savePassword: boolean;
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

export interface FieldError {
  field: string;
  message: string;
}

export interface CommandError {
  code: string;
  message: string;
  fields: FieldError[];
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
