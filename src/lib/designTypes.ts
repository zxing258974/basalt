// 表结构设计,与 crates/basalt-core/src/design 的序列化结构一一对应。
import type { TableRef } from "./types";

export interface Generated {
  expression: string;
  /** 存储列(STORED);否则是虚拟列。 */
  stored: boolean;
}

export interface ColumnDef {
  /** 读出来的是 `col:原名`,设计器里新加的是 `new:…`。按它配对,改名才不会变成删了再加。 */
  id: string;
  name: string;
  /** 完整类型,如 varchar(50);SQLite 可以为空。 */
  dataType: string;
  nullable: boolean;
  /** 默认值的 SQL 表达式原文,如 'abc'、0、CURRENT_TIMESTAMP。 */
  default: string | null;
  autoIncrement: boolean;
  comment: string | null;
  /** 以下三项设计器不改,原样带回去。 */
  collation: string | null;
  onUpdate: string | null;
  generated: Generated | null;
}

export interface IndexColumn {
  /** 字段 id。 */
  column: string;
  descending: boolean;
  length: number | null;
}

export interface IndexDef {
  id: string;
  name: string;
  columns: IndexColumn[];
  unique: boolean;
  /** MySQL 的 FULLTEXT / SPATIAL / HASH,PostgreSQL 的 gin、gist……;null 是默认的 B 树。 */
  method: string | null;
  /** 表达式索引、部分索引的原始定义:只能删除。 */
  definition: string | null;
  /** 唯一约束带出来的索引。 */
  isConstraint: boolean;
}

export type FkAction = "noAction" | "restrict" | "cascade" | "setNull" | "setDefault";

export interface ForeignKeyDef {
  id: string;
  /** 为空时由数据库命名。 */
  name: string;
  /** 本表的字段 id。 */
  columns: string[];
  /** 被引用表所在的库(MySQL)或 schema(PostgreSQL);null 表示和本表相同。 */
  refSchema: string | null;
  refTable: string;
  /** 被引用的字段名。 */
  refColumns: string[];
  onUpdate: FkAction;
  onDelete: FkAction;
}

export interface CheckDef {
  id: string;
  name: string | null;
  expression: string;
}

export interface TriggerDef {
  name: string;
  sql: string;
}

export interface TableOptions {
  engine: string | null;
  charset: string | null;
  collation: string | null;
  comment: string | null;
  withoutRowid: boolean;
  strict: boolean;
}

export interface TableDesign {
  table: TableRef;
  columns: ColumnDef[];
  /** 主键里的字段 id,按主键顺序。 */
  primaryKey: string[];
  primaryKeyName: string | null;
  indexes: IndexDef[];
  foreignKeys: ForeignKeyDef[];
  /** 只读。 */
  checks: CheckDef[];
  /** 只读,SQLite 才有。 */
  triggers: TriggerDef[];
  options: TableOptions;
}

/** transaction:一个事务里执行;autocommit:MySQL 逐条执行;rebuild:SQLite 重建表。 */
export type ExecMode = "transaction" | "autocommit" | "rebuild";

export interface DdlPreview {
  statements: string[];
  mode: ExecMode;
}

export interface DdlOutcome {
  statements: number;
  /** 执行后的表(改名后是新名字)。 */
  table: TableRef;
}
