// mock 后端的 SQL 编辑器:认得几种常见语句,够在浏览器里调界面、在单测里走完整流程。
// SELECT … FROM 某张示例表返回那张表的数据;SLEEP(n) / pg_sleep(n) 会一直跑到被取消。
import type {
  HistoryEntry,
  OutcomeStatus,
  QueryImportCandidate,
  QueryImportReport,
  QueryScope,
  ResultSet,
  SavedQueryInfo,
  SavedQueryRef,
  StatementOutcome,
} from "../queryTypes";
import type { CellValue, CommandError, ConnectionConfig } from "../types";
import { seedTable } from "./seedTables";

const MAX_ROWS = 5000;
/** 示例表里叫这些名字的当作不存在,用来演示报错。 */
const MISSING_TABLE = /^(missing|no_such\w*|nope)$/i;

function fail(code: string, message: string): never {
  const error: CommandError = { code, message, fields: [] };
  throw error;
}

const keyword = (sql: string) =>
  /^[\s(]*([A-Za-z]+)/.exec(sql.replace(/^(\s*(--[^\n]*\n|#[^\n]*\n|\/\*[\s\S]*?\*\/))*/, ""))?.[1]?.toUpperCase() ??
  "";

function outcome(scope: QueryScope, partial: Partial<StatementOutcome>): StatementOutcome {
  return {
    status: "ok",
    message: null,
    resultSets: [],
    affectedRows: null,
    elapsedMs: 3,
    scope,
    notice: null,
    ...partial,
  };
}

function literalCell(expression: string): CellValue {
  const trimmed = expression.trim();
  if (/^-?\d+$/.test(trimmed)) return { kind: "int", value: trimmed };
  if (/^-?\d+\.\d+$/.test(trimmed)) return { kind: "decimal", value: trimmed };
  if (/^null$/i.test(trimmed)) return { kind: "null" };
  const quoted = /^'(.*)'$/s.exec(trimmed);
  return { kind: "text", value: quoted ? quoted[1]!.replaceAll("''", "'") : trimmed };
}

/** 没有 FROM 的 SELECT:每个逗号分开的表达式一列,`AS 别名` 当列名。 */
function selectLiterals(list: string): ResultSet {
  const parts = list.split(",").map((part) => {
    const alias = /^(.*?)\s+(?:as\s+)?([A-Za-z_]\w*)$/is.exec(part.trim());
    return alias && !/^'/.test(alias[2]!) ? { expr: alias[1]!, name: alias[2]! } : { expr: part, name: part.trim() };
  });
  return {
    columns: parts.map((p) => ({ name: p.name, dataType: "" })),
    rows: [parts.map((p) => literalCell(p.expr))],
    truncated: false,
  };
}

function select(sql: string, scope: QueryScope): StatementOutcome {
  const from = /\bfrom\s+(?:[`"]?\w+[`"]?\.)?[`"]?(\w+)[`"]?/i.exec(sql);
  if (!from) {
    const list = /^\s*select\s+([\s\S]*?)\s*;?\s*$/i.exec(sql)?.[1] ?? "1";
    return outcome(scope, { resultSets: [selectLiterals(list)] });
  }
  const name = from[1]!;
  if (MISSING_TABLE.test(name)) {
    return outcome(scope, {
      status: "error",
      message: `1146 - Table '${scope.database ?? "db"}.${name}' doesn't exist`,
    });
  }
  const table = seedTable(name);
  const limit = Number(/\blimit\s+(\d+)/i.exec(sql)?.[1] ?? Number.POSITIVE_INFINITY);
  const empty = /\bwhere\s+1\s*=\s*0\b/i.test(sql);
  const rows = empty ? [] : table.rows.slice(0, Math.min(limit, MAX_ROWS));
  return outcome(scope, {
    resultSets: [
      { columns: table.columns.map((c) => ({ name: c.name, dataType: c.dataType })), rows, truncated: false },
    ],
    elapsedMs: 12,
  });
}

type SavedEntry = { sql: string; modifiedAt: string };

const savedKey = (q: Pick<SavedQueryRef, "connectionId" | "database" | "schema">, name: string) =>
  [q.connectionId, q.database, q.schema ?? "", name].join("\u0000");

const SEED_SAVED: [SavedQueryRef, string][] = [
  [
    { connectionId: "mock-mysql", database: "shop", schema: null, name: "每日销售" },
    "SELECT * FROM v_daily_sales ORDER BY day DESC;",
  ],
  [
    { connectionId: "mock-mysql", database: "shop", schema: null, name: "大客户" },
    "-- 消费最多的客户\nSELECT * FROM customers LIMIT 20;\nSELECT * FROM orders LIMIT 20;",
  ],
  [
    { connectionId: "mock-pg", database: "folio", schema: "diku_mod_inventory_storage", name: "馆藏统计" },
    "SELECT count(*) AS n FROM item;",
  ],
];

const NAVICAT_QUERIES: QueryImportCandidate[] = [
  {
    id: "0/0/MySQL/本地 MySQL/shop/退款明细.sql",
    name: "退款明细",
    sourceType: "MySQL",
    sourceConnection: "本地 MySQL",
    sourceDatabase: "shop",
    sourceSchema: null,
    target: {
      connectionId: "mock-mysql",
      connectionName: "本地 MySQL",
      database: "shop",
      schema: null,
      name: "退款明细",
    },
    matchedBy: "name",
    status: { kind: "new" },
  },
  {
    id: "0/0/PostgreSQL/folio-test/folio/diku_mod_users/读者.sql",
    name: "读者",
    sourceType: "PostgreSQL",
    sourceConnection: "folio-test",
    sourceDatabase: "folio",
    sourceSchema: "diku_mod_users",
    target: {
      connectionId: "mock-pg",
      connectionName: "FOLIO 测试库",
      database: "folio",
      schema: "diku_mod_users",
      name: "读者",
    },
    matchedBy: "address",
    status: { kind: "new" },
  },
  {
    id: "0/0/MySQL/aliyun/erm/核心期刊.sql",
    name: "核心期刊",
    sourceType: "MySQL",
    sourceConnection: "aliyun",
    sourceDatabase: "erm",
    sourceSchema: null,
    target: null,
    matchedBy: null,
    status: { kind: "unmatched", reason: "Basalt 里没有叫「aliyun」的 MySQL 连接，也没有地址是 10.0.0.5:3306 的连接" },
  },
];

export function createMockQueries(connections: () => ConnectionConfig[], wait: (ms?: number) => Promise<unknown>) {
  const history: HistoryEntry[] = [];
  const saved = new Map<string, SavedEntry>(
    SEED_SAVED.map(([ref, sql]) => [savedKey(ref, ref.name), { sql, modifiedAt: "2026-10-18T09:30:00+08:00" }]),
  );
  /** 正在 sleep 的语句:取消时调用,让它提前结束。 */
  const sleeping = new Map<string, () => void>();

  const sleep = (editorId: string, seconds: number) =>
    new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => {
        sleeping.delete(editorId);
        resolve(false);
      }, seconds * 1000);
      sleeping.set(editorId, () => {
        clearTimeout(timer);
        sleeping.delete(editorId);
        resolve(true);
      });
    });

  async function execute(editorId: string, scope: QueryScope, sql: string): Promise<StatementOutcome> {
    const seconds = /(?:pg_)?sleep\s*\(\s*(\d+(?:\.\d+)?)/i.exec(sql);
    if (seconds) {
      const cancelled = await sleep(editorId, Math.min(Number(seconds[1]), 60));
      return cancelled
        ? outcome(scope, { status: "cancelled", message: "Query execution was interrupted", elapsedMs: 800 })
        : outcome(scope, { resultSets: [selectLiterals("0 AS slept")] });
    }
    const word = keyword(sql);
    if (word === "SELECT" || word === "WITH" || word === "SHOW") return select(sql, scope);
    if (word === "USE") return outcome({ ...scope, database: /use\s+[`"]?(\w+)/i.exec(sql)?.[1] ?? null }, {});
    if (["INSERT", "UPDATE", "DELETE", "REPLACE"].includes(word)) {
      const groups = sql.match(/\)\s*,\s*\(/g)?.length ?? 0;
      return outcome(scope, { affectedRows: word === "INSERT" ? groups + 1 : 3 });
    }
    if (!word) return outcome(scope, { status: "error", message: "1065 - Query was empty" });
    return outcome(scope, {});
  }

  function record(connectionId: string, scope: QueryScope, sql: string, result: StatementOutcome) {
    const connection = connections().find((c) => c.id === connectionId);
    const rows = result.resultSets.reduce((sum, set) => sum + set.rows.length, 0);
    history.push({
      id: crypto.randomUUID(),
      executedAt: new Date().toISOString(),
      connectionId,
      connectionName: connection?.name ?? connectionId,
      database: scope.database,
      schema: scope.schema,
      sql,
      status: result.status satisfies OutcomeStatus,
      elapsedMs: result.elapsedMs,
      affectedRows: result.affectedRows,
      rowCount: result.resultSets.length > 0 ? rows : null,
    });
  }

  const find = (ref: SavedQueryRef) => saved.get(savedKey(ref, ref.name.trim()));

  return {
    async runStatement(editorId: string, connectionId: string, scope: QueryScope, sql: string) {
      await wait(40);
      const result = await execute(editorId, scope, sql);
      record(connectionId, scope, sql, result);
      return result;
    },
    async cancelStatement(editorId: string) {
      await wait(20);
      const stop = sleeping.get(editorId);
      stop?.();
      return stop !== undefined;
    },
    async closeEditor(editorId: string) {
      sleeping.get(editorId)?.();
      await wait(10);
    },
    async searchHistory(query: string, connectionId: string | null, limit: number) {
      await wait(30);
      const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
      return history
        .filter((e) => !connectionId || e.connectionId === connectionId)
        .filter((e) =>
          terms.every((t) => `${e.sql}\n${e.connectionName}\n${e.database ?? ""}`.toLowerCase().includes(t)),
        )
        .reverse()
        .slice(0, limit);
    },
    async clearHistory() {
      await wait(20);
      history.length = 0;
    },
    async listSavedQueries(connectionId: string, database: string, schema: string | null): Promise<SavedQueryInfo[]> {
      await wait(40);
      const prefix = savedKey({ connectionId, database, schema }, "");
      return [...saved.entries()]
        .filter(([key]) => key.startsWith(prefix))
        .map(([key, entry]) => ({ name: key.slice(prefix.length), modifiedAt: entry.modifiedAt }))
        .sort((a, b) => a.name.localeCompare(b.name, "zh-CN"));
    },
    async readSavedQuery(ref: SavedQueryRef) {
      await wait(30);
      return find(ref)?.sql ?? fail("not_found", `找不到查询「${ref.name}」，可能已被删除或改名`);
    },
    async saveQuery(ref: SavedQueryRef, sql: string, overwrite: boolean): Promise<SavedQueryInfo> {
      await wait(40);
      const name = ref.name.trim();
      if (!name) fail("invalid_name", "请给查询起个名字");
      if (/[/\\:*?"<>|]/.test(name) || name.startsWith("."))
        fail("invalid_name", '名字里不能有 / \\ : * ? " < > | 或换行这类控制字符');
      if (!overwrite && find(ref)) fail("already_exists", `这个库下已经有叫「${name}」的查询了`);
      const modifiedAt = new Date().toISOString();
      saved.set(savedKey(ref, name), { sql, modifiedAt });
      return { name, modifiedAt };
    },
    async deleteSavedQuery(ref: SavedQueryRef) {
      await wait(30);
      saved.delete(savedKey(ref, ref.name.trim()));
    },
    async scanNavicatQueries(): Promise<QueryImportCandidate[]> {
      await wait();
      return NAVICAT_QUERIES.map((candidate) =>
        candidate.target && find(candidate.target) ? { ...candidate, status: { kind: "exists" } } : candidate,
      );
    },
    async importNavicatQueries(ids: string[]): Promise<QueryImportReport> {
      await wait(200);
      const report: QueryImportReport = { imported: [], skipped: [] };
      for (const id of ids) {
        const candidate = NAVICAT_QUERIES.find((c) => c.id === id);
        if (!candidate?.target || candidate.status.kind !== "new" || find(candidate.target)) {
          report.skipped.push({ name: candidate?.name ?? id, reason: "已经导入过了" });
          continue;
        }
        saved.set(savedKey(candidate.target, candidate.target.name), {
          sql: `-- 从 Navicat 导入\nSELECT * FROM ${candidate.target.database === "shop" ? "orders" : "users"} LIMIT 50;`,
          modifiedAt: new Date().toISOString(),
        });
        report.imported.push({ name: candidate.name, target: candidate.target });
      }
      return report;
    },
  };
}
