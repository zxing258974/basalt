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

/** 清空表:delete 是 DELETE FROM(保留自增计数),truncate 是 TRUNCATE(自增计数从头开始)。 */
export type ClearMode = "delete" | "truncate";

/** 清空表的结果。rows:删了多少行;MySQL、PostgreSQL 的截断不报行数,为 null。 */
export interface ClearTableOutcome {
  rows: number | null;
}

/** 外键的 ON DELETE 规则。 */
export type OnDelete = "cascade" | "setNull" | "setDefault" | "restrict" | "noAction";

/** 别的表上引用这张表的外键:清空时按规则连带改动那张表,截断时会被拒绝。name:SQLite 的外键没有名字。 */
export interface ReferencingKey {
  table: TableRef;
  name: string | null;
  onDelete: OnDelete;
}

/** 表的维护,各家能做的不一样(见 tableOpsModel 的 MAINTENANCE)。 */
export type Maintenance = "analyze" | "check" | "optimize" | "repair" | "vacuum" | "reindex";

/** 数据库返回的一条消息。level 是 MySQL 的 Msg_type:status、error、info、note、warning。 */
export interface MaintenanceMessage {
  level: string;
  text: string;
}

/** 维护的结果。PostgreSQL、SQLite 不返回消息;size 只有 VACUUM、REINDEX 才有(字节)。 */
export interface MaintenanceOutcome {
  sql: string;
  messages: MaintenanceMessage[];
  size: { before: number; after: number } | null;
  elapsedMs: number;
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
