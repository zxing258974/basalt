// mock 后端的导入来源:几条虚构的 Navicat 连接,以及和 Rust 端一致的「已存在」判定。
import type { ImportCandidate, ImportReport, ImportSource } from "./importTypes";
import type { CommandError, ConnectionConfig, Endpoint } from "./types";

type MockSource = Omit<ImportCandidate, "status"> & { unsupportedReason?: string };

const TUNNEL_SSL_WARNING = "隧道 / SSL 设置未迁移（M4 支持）";
const UNSUPPORTED = (label: string) => `Basalt 暂不支持 ${label}，目前只支持 MySQL、MariaDB、PostgreSQL 和 SQLite`;

const net = (host: string, port: number, username: string, database?: string): Endpoint => ({
  kind: "network",
  host,
  port,
  username,
  database: database ?? null,
});

function source(id: string, partial: Partial<MockSource> & Pick<MockSource, "name">): MockSource {
  return {
    id,
    group: null,
    sourceType: "MySQL",
    driver: "mysql",
    endpoint: null,
    password: "absent",
    warnings: [],
    ...partial,
  };
}

const LOCAL_NAVICAT: MockSource[] = [
  source("MySQL:本地 MySQL", { name: "本地 MySQL", endpoint: net("127.0.0.1", 3306, "root") }),
  source("MySQL:订单库", { name: "订单库", endpoint: net("mysql.example.test", 3306, "app") }),
  source("MySQL:测试库", {
    name: "测试库",
    endpoint: net("mysql-test.example.test", 13306, "tester"),
    warnings: [TUNNEL_SSL_WARNING],
  }),
  source("PostgreSQL:分析库", {
    name: "分析库",
    sourceType: "PostgreSQL",
    driver: "postgres",
    endpoint: net("pg.example.test", 5432, "analyst", "warehouse"),
  }),
  source("REDIS:会话缓存", {
    name: "会话缓存",
    sourceType: "Redis",
    driver: null,
    unsupportedReason: UNSUPPORTED("Redis"),
  }),
];

const NCX_EXPORT: MockSource[] = [
  source("MYSQL:订单库", { name: "订单库", endpoint: net("mysql.example.test", 3306, "app"), password: "included" }),
  source("POSTGRESQL:报表 PG", {
    name: "报表 PG",
    sourceType: "PostgreSQL",
    driver: "postgres",
    endpoint: net("192.0.2.10", 5433, "report_ro", "reports"),
    password: "included",
    warnings: [TUNNEL_SSL_WARNING],
  }),
  source("MYSQL:旧版导出", {
    name: "旧版导出",
    endpoint: net("legacy.example.test", 3306, "legacy"),
    password: "undecryptable",
  }),
  source("SQLITE:本地笔记", {
    name: "本地笔记",
    sourceType: "SQLite",
    driver: "sqlite",
    endpoint: { kind: "file", path: "/Users/me/Documents/notes-2025.db" },
  }),
  source("SQLSERVER:财务", {
    name: "财务",
    sourceType: "SQL Server",
    driver: null,
    password: "included",
    unsupportedReason: UNSUPPORTED("SQL Server"),
  }),
];

export function mockImportSources(source: ImportSource): MockSource[] {
  if (source.kind === "localNavicat") return LOCAL_NAVICAT;
  if (!source.path.endsWith(".ncx")) {
    const error: CommandError = {
      code: "unsupported",
      message: "不是有效的 Navicat 导出文件（.ncx）：没有 <Connections> 根元素",
      fields: [],
    };
    throw error;
  }
  return NCX_EXPORT;
}

function sameAddress(a: Endpoint, b: Endpoint): boolean {
  if (a.kind === "file" || b.kind === "file") {
    return a.kind === "file" && b.kind === "file" && a.path === b.path;
  }
  return a.host.toLowerCase() === b.host.toLowerCase() && a.port === b.port && a.username === b.username;
}

/** 判重用的连接要素;本次来源里的重复项没有 id,只能跳过、不能补密码。 */
type Known = Pick<ConnectionConfig, "name" | "driver" | "endpoint" | "savePassword"> & { id: string | null };

/** 与 Rust 端一致:同一来源里后出现的重复项也算已存在;只有地址对得上、来源带密码、
 *  已有连接还没存密码时才「补密码」,只靠名字对上的绝不补。 */
export function classifyMockImport(
  sources: readonly MockSource[],
  existing: readonly ConnectionConfig[],
): ImportCandidate[] {
  let known: Known[] = existing.map((c) => ({ ...c, id: c.id }));
  return sources.map(({ unsupportedReason, ...rest }) => {
    if (unsupportedReason || !rest.driver || !rest.endpoint) {
      return {
        ...rest,
        driver: null,
        endpoint: null,
        status: { kind: "unsupported", reason: unsupportedReason ?? "" },
      };
    }
    const { driver, endpoint } = rest;
    const same = (c: Known) => c.driver === driver && sameAddress(c.endpoint, endpoint);
    const match = known.find((c) => same(c) || c.name === rest.name);
    known = [...known, { id: null, name: rest.name, driver, endpoint, savePassword: false }];
    if (!match) return { ...rest, status: { kind: "new" } };
    const fillable =
      match.id !== null &&
      same(match) &&
      endpoint.kind === "network" &&
      rest.password === "included" &&
      !match.savePassword;
    return {
      ...rest,
      status: fillable
        ? { kind: "fillPassword", existingId: match.id!, existingName: match.name }
        : { kind: "existing", existingName: match.name },
    };
  });
}

/** 模拟导入:返回要追加的连接、要补密码的已有连接 id 和汇报。 */
export function planMockImport(
  sources: readonly MockSource[],
  ids: readonly string[],
  existing: readonly ConnectionConfig[],
): { configs: ConnectionConfig[]; filledIds: string[]; report: ImportReport } {
  const wanted = new Set(ids);
  const statuses = new Map(classifyMockImport(sources, existing).map((c) => [c.id, c.status]));
  const picked = sources.filter((s) => wanted.has(s.id));
  const fills = picked.flatMap((s) => {
    const status = statuses.get(s.id);
    return status?.kind === "fillPassword" ? [{ id: status.existingId, name: status.existingName, source: s.id }] : [];
  });
  const filledSources = new Set(fills.map((f) => f.source));
  const importable = picked.filter((s) => !filledSources.has(s.id) && !s.unsupportedReason && s.driver && s.endpoint);
  const configs: ConnectionConfig[] = importable.map((s) => ({
    id: crypto.randomUUID(),
    name: s.name,
    group: s.group,
    driver: s.driver!,
    endpoint: s.endpoint!,
    savePassword: s.password === "included",
  }));
  const known = new Set(sources.map((s) => s.id));
  const report: ImportReport = {
    imported: configs.map((c, i) => ({
      id: c.id,
      name: c.name,
      driver: c.driver,
      passwordSaved: importable[i]!.password === "included",
    })),
    passwordFilled: fills.map(({ id, name }) => ({ id, name })),
    skipped: [
      ...picked
        .filter((s) => !importable.includes(s) && !filledSources.has(s.id))
        .map((s) => ({ name: s.name, reason: s.unsupportedReason ?? "无法导入" })),
      ...ids
        .filter((id) => !known.has(id))
        .map((id) => ({ name: id, reason: "扫描之后来源有变化，找不到这个连接，请重新扫描" })),
    ],
  };
  return { configs, filledIds: fills.map((f) => f.id), report };
}
