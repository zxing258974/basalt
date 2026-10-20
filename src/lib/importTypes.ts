// 导入连接,与 crates/basalt-core/src/import 和 src-tauri/src/service/import.rs 的序列化结构对应。
// 这里没有任何明文密码:预览只说「含不含密码」,密码由后端直接写进钥匙串。
import type { DriverKind, Endpoint } from "./types";

export type ImportSource = { kind: "localNavicat" } | { kind: "ncxFile"; path: string };

export type CandidateStatus =
  | { kind: "new" }
  | { kind: "existing"; existingName: string }
  /** 同一个库已经在 Basalt 里、但还没存密码:导入时只把来源里的密码补过去,不新建连接。 */
  | { kind: "fillPassword"; existingId: string; existingName: string }
  | { kind: "unsupported"; reason: string };

export type PasswordStatus = "included" | "absent" | "undecryptable";

export interface ImportCandidate {
  /** 同一来源内唯一,导入时按它回传勾选项。 */
  id: string;
  name: string;
  group: string | null;
  /** 来源里的类型名,如 MySQL、Redis。 */
  sourceType: string;
  driver: DriverKind | null;
  endpoint: Endpoint | null;
  status: CandidateStatus;
  password: PasswordStatus;
  /** 照常导入、但没有迁移过来的设置。 */
  warnings: string[];
}

export interface ImportedConnection {
  id: string;
  name: string;
  driver: DriverKind;
  passwordSaved: boolean;
}

export interface SkippedConnection {
  name: string;
  reason: string;
}

/** 没有新建、只给已有连接补上了密码。 */
export interface FilledConnection {
  id: string;
  name: string;
}

export interface ImportReport {
  imported: ImportedConnection[];
  passwordFilled: FilledConnection[];
  skipped: SkippedConnection[];
}
