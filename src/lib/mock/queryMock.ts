// mock 后端的 SQL 编辑器:认得几种常见语句,够在浏览器里调界面、在单测里走完整流程。
// SELECT … FROM 某张示例表返回那张表的数据;SLEEP(n) / pg_sleep(n) 会一直跑到被取消。
import type {
  HistoryEntry,
  OutcomeStatus,
  QueryImportCandidate,
  QueryImportReport,
  QueryScope,
  ResultColumn,
  ResultSet,
  ResultSort,
  SavedQueryInfo,
  SavedQueryRef,
  StatementOutcome,
} from "../queryTypes";
import type { CellValue, CommandError, ConnectionConfig } from "../types";
import { seedTable } from "./seedTables";

/** 执行完随结果带回的行数,和后端一样;其余的往下滚时再取。 */
const FIRST_BATCH = 1000;
const MAX_BATCH = 5000;
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

const NUMERIC = new Set<CellValue["kind"]>(["int", "uint", "float", "decimal"]);

/** 和后端的排序一样:NULL 排最前;数字按数值比,其余按文本比(中文按拼音)。 */
export function compareCells(a: CellValue, b: CellValue): number {
  if (a.kind === "null" || b.kind === "null") return a.kind === b.kind ? 0 : a.kind === "null" ? -1 : 1;
  if (NUMERIC.has(a.kind) && NUMERIC.has(b.kind) && "value" in a && "value" in b) {
    const diff = Number(a.value) - Number(b.value);
    if (diff !== 0 && Number.isFinite(diff)) return Math.sign(diff);
  }
  const text = (cell: CellValue) => ("value" in cell ? String(typeof cell.value === "object" ? "" : cell.value) : "");
  return text(a).localeCompare(text(b), "zh-CN", { numeric: true });
}

/** 稳定排序:值相同的行保持原来的先后。 */
function sortRows(rows: readonly CellValue[][], sort: ResultSort): CellValue[][] {
  return rows
    .map((row, at) => ({ row, at }))
    .sort((x, y) => compareCells(x.row[sort.column]!, y.row[sort.column]!) * (sort.descending ? -1 : 1) || x.at - y.at)
    .map(({ row }) => row);
}

/** 执行出来的完整结果,结果集只带第一批行;存进标签页的结果里由 runStatement 补上编号。 */
type FullSet = { columns: ResultColumn[]; rows: CellValue[][] };

/** 没有 FROM 的 SELECT:每个逗号分开的表达式一列,`AS 别名` 当列名。 */
function selectLiterals(list: string): FullSet {
  const parts = list.split(",").map((part) => {
    const alias = /^(.*?)\s+(?:as\s+)?([A-Za-z_]\w*)$/is.exec(part.trim());
    return alias && !/^'/.test(alias[2]!) ? { expr: alias[1]!, name: alias[2]! } : { expr: part, name: part.trim() };
  });
  return {
    columns: parts.map((p) => ({ name: p.name, dataType: "", source: null })),
    rows: [parts.map((p) => literalCell(p.expr))],
  };
}

type Executed = { outcome: StatementOutcome; sets: FullSet[] };

const done = (outcome: StatementOutcome, sets: FullSet[] = []): Executed => ({ outcome, sets });

/** SELECT … FROM 示例表:列都带来源,`*` 以外的列表只认 `列`、`列 AS 别名`,别的当表达式。 */
function select(sql: string, scope: QueryScope): Executed {
  const from = /\bfrom\s+(?:[`"]?\w+[`"]?\.)?[`"]?(\w+)[`"]?/i.exec(sql);
  if (!from) {
    const list = /^\s*select\s+([\s\S]*?)\s*;?\s*$/i.exec(sql)?.[1] ?? "1";
    return done(outcome(scope, {}), [selectLiterals(list)]);
  }
  const name = from[1]!;
  if (MISSING_TABLE.test(name)) {
    return done(
      outcome(scope, {
        status: "error",
        message: `1146 - Table '${scope.database ?? "db"}.${name}' doesn't exist`,
      }),
    );
  }
  const table = seedTable(name);
  const ref = { database: scope.database ?? "", schema: scope.schema, table: name };
  const limit = Number(/\blimit\s+(\d+)/i.exec(sql)?.[1] ?? Number.POSITIVE_INFINITY);
  const empty = /\bwhere\s+1\s*=\s*0\b/i.test(sql);
  const rows = empty ? [] : table.rows.slice(0, limit);
  const list = /^\s*select\s+([\s\S]*?)\s+from\b/i.exec(sql)?.[1]?.trim() ?? "*";
  const picked =
    list === "*"
      ? table.columns.map((c, index) => ({ index, name: c.name, column: c.name }))
      : list.split(",").map((part) => {
          const [, expr = "", alias] = /^(.*?)(?:\s+as\s+([A-Za-z_]\w*))?$/is.exec(part.trim()) ?? [];
          const bare = expr.replace(/^\w+\./, "").replace(/[`"]/g, "");
          const index = table.columns.findIndex((c) => c.name === bare);
          return { index, name: alias ?? expr, column: index >= 0 ? bare : null };
        });
  const columns = picked.map((p): ResultColumn => ({
    name: p.name,
    dataType: p.index >= 0 ? table.columns[p.index]!.dataType : "",
    source: p.column !== null ? { table: ref, column: p.column } : null,
  }));
  const values = rows.map((row) => picked.map((p): CellValue => (p.index >= 0 ? row[p.index]! : { kind: "null" })));
  return done(outcome(scope, { elapsedMs: 12 }), [{ columns, rows: values }]);
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
    id: "0/0/MySQL/cloud-mysql/crm/月度销售.sql",
    name: "月度销售",
    sourceType: "MySQL",
    sourceConnection: "cloud-mysql",
    sourceDatabase: "crm",
    sourceSchema: null,
    target: null,
    matchedBy: null,
    status: {
      kind: "unmatched",
      reason: "Basalt 里没有叫「cloud-mysql」的 MySQL 连接，也没有地址是 10.0.0.5:3306 的连接",
    },
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

  /** 每个查询标签页存着的完整结果,按编号取。 */
  const stored = new Map<string, Map<number, CellValue[][]>>();
  let nextResultId = 0;

  /** 完整结果存起来,结果集里只带第一批行。 */
  function keep(editorId: string, sets: FullSet[]): ResultSet[] {
    const kept = stored.get(editorId) ?? new Map<number, CellValue[][]>();
    stored.set(editorId, kept);
    return sets.map((set) => {
      nextResultId += 1;
      kept.set(nextResultId, set.rows);
      return {
        id: nextResultId,
        columns: set.columns,
        rows: set.rows.slice(0, FIRST_BATCH),
        totalRows: set.rows.length,
        truncated: false,
        readOnly: null,
      };
    });
  }

  async function execute(editorId: string, scope: QueryScope, sql: string): Promise<Executed> {
    const seconds = /(?:pg_)?sleep\s*\(\s*(\d+(?:\.\d+)?)/i.exec(sql);
    if (seconds) {
      const cancelled = await sleep(editorId, Math.min(Number(seconds[1]), 60));
      return cancelled
        ? done(outcome(scope, { status: "cancelled", message: "Query execution was interrupted", elapsedMs: 800 }))
        : done(outcome(scope, {}), [selectLiterals("0 AS slept")]);
    }
    const word = keyword(sql);
    if (word === "SELECT" || word === "WITH" || word === "SHOW") return select(sql, scope);
    if (word === "USE") return done(outcome({ ...scope, database: /use\s+[`"]?(\w+)/i.exec(sql)?.[1] ?? null }, {}));
    if (["INSERT", "UPDATE", "DELETE", "REPLACE"].includes(word)) {
      const groups = sql.match(/\)\s*,\s*\(/g)?.length ?? 0;
      return done(outcome(scope, { affectedRows: word === "INSERT" ? groups + 1 : 3 }));
    }
    if (!word) return done(outcome(scope, { status: "error", message: "1065 - Query was empty" }));
    return done(outcome(scope, {}));
  }

  function record(connectionId: string, scope: QueryScope, sql: string, result: StatementOutcome) {
    const connection = connections().find((c) => c.id === connectionId);
    const rows = result.resultSets.reduce((sum, set) => sum + set.totalRows, 0);
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
      const executed = await execute(editorId, scope, sql);
      const result = { ...executed.outcome, resultSets: keep(editorId, executed.sets) };
      record(connectionId, scope, sql, result);
      return result;
    },
    async resultRows(editorId: string, resultId: number, offset: number, limit: number, sort: ResultSort | null) {
      await wait(30);
      const rows = stored.get(editorId)?.get(resultId);
      if (!rows) fail("not_found", "这个结果已经释放了，请重新执行。");
      const ordered = sort ? sortRows(rows, sort) : rows;
      return ordered.slice(offset, offset + Math.min(limit, MAX_BATCH));
    },
    async releaseResults(editorId: string, resultIds: number[]) {
      await wait(10);
      for (const id of resultIds) stored.get(editorId)?.delete(id);
    },
    /** 导出时报的行数。 */
    resultSize: (editorId: string, resultId: number) => stored.get(editorId)?.get(resultId)?.length,
    async cancelStatement(editorId: string) {
      await wait(20);
      const stop = sleeping.get(editorId);
      stop?.();
      return stop !== undefined;
    },
    async closeEditor(editorId: string) {
      sleeping.get(editorId)?.();
      stored.delete(editorId);
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
