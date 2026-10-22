// mock 后端的服务器监控:每个连接一台假服务器,计数器按时间往上涨(速率随快照次数起伏),
// 会话列表里有在跑的、在等锁的、空闲的连接池和后台线程。终止会话会真的改掉列表,
// 和后端一样先核对会话还是确认时的样子。
import type { KillTarget, ProcessActivity, ServerCounters, ServerProcess, ServerSnapshot } from "../monitorTypes";
import type { CommandError, DriverKind } from "../types";

function fail(code: string, message: string): never {
  const error: CommandError = { code, message, fields: [] };
  throw error;
}

/** 会话的样子;`since` 是进入当前状态的时刻,秒数按它现算。 */
interface MockProcess extends Omit<ServerProcess, "seconds"> {
  since: number;
}

interface Server {
  driver: DriverKind;
  startedAt: number;
  /** 计数器带小数累加,给出去时取整。 */
  totals: Record<"queries" | "commits" | "rollbacks" | "received" | "sent" | "reads" | "misses", number>;
  slow: number;
  takenAt: number;
  snapshots: number;
  processes: MockProcess[];
}

const UPTIME_SECONDS = 3 * 86_400 + 4 * 3_600 + 17 * 60;

type Seed = [
  id: number,
  user: string,
  host: string | null,
  database: string | null,
  state: string,
  detail: string | null,
  agoSeconds: number,
  query: string | null,
];

const MYSQL_SEEDS: Seed[] = [
  [5, "event_scheduler", "localhost", null, "Daemon", "Waiting on empty queue", 86_000, null],
  [
    1184,
    "app",
    "10.0.8.31:40112",
    "shop",
    "Query",
    "Sending data",
    12,
    "SELECT o.id, c.name, SUM(oi.price * oi.qty) AS total FROM orders o JOIN customers c ON c.id = o.customer_id JOIN order_items oi ON oi.order_id = o.id WHERE o.created_at >= '2026-01-01' GROUP BY o.id, c.name ORDER BY total DESC",
  ],
  [
    1191,
    "app",
    "10.0.8.31:40118",
    "shop",
    "Query",
    "Waiting for table metadata lock",
    4,
    "ALTER TABLE order_items ADD COLUMN note varchar(200)",
  ],
  [1203, "report", "10.0.8.40:51022", "analytics", "Sleep", null, 1_260, null],
  [1210, "root", "127.0.0.1:52190", "shop", "Query", "init", 0, "SHOW FULL PROCESSLIST"],
];

const PG_SEEDS: Seed[] = [
  [
    41822,
    "folio",
    "10.0.8.31:40112",
    "folio",
    "active",
    null,
    9,
    "SELECT jsonb FROM diku_mod_inventory_storage.instance WHERE lower(f_unaccent(jsonb->>'title')) LIKE '%history%' ORDER BY jsonb->>'title' LIMIT 1000",
  ],
  [
    41830,
    "folio",
    "10.0.8.31:40118",
    "folio",
    "active",
    "Lock:relation",
    3,
    "VACUUM FULL diku_mod_inventory_storage.item",
  ],
  [
    41851,
    "folio",
    "10.0.8.33:38812",
    "folio",
    "idle in transaction",
    "Client:ClientRead",
    74,
    "UPDATE diku_mod_users.users SET jsonb = $1 WHERE id = $2",
  ],
  [41868, "okapi", "10.0.8.20:55102", "okapi", "idle", "Client:ClientRead", 620, "SELECT 1"],
  [
    41902,
    "folio",
    "127.0.0.1:52190",
    "folio",
    "active",
    null,
    0,
    "SELECT pid::int8, usename::text, … FROM pg_stat_activity",
  ],
];

/** 连接池里闲着的连接。 */
function poolSeeds(driver: DriverKind): Seed[] {
  const mysql = driver !== "postgres";
  return Array.from({ length: 8 }, (_, i) => [
    mysql ? 1100 + i * 7 : 41700 + i * 11,
    mysql ? "app" : "folio",
    `10.0.8.${31 + (i % 3)}:${40200 + i}`,
    mysql ? "shop" : "folio",
    mysql ? "Sleep" : "idle",
    mysql ? null : "Client:ClientRead",
    3 + i * 37,
    mysql ? null : "COMMIT",
  ]);
}

/** 和后端一样按状态分:MySQL 看 Command 和用户,PostgreSQL 看 state。 */
function activityOf(driver: DriverKind, state: string, user: string): ProcessActivity {
  if (driver === "postgres") {
    if (state === "active") return "running";
    if (state.startsWith("idle in transaction")) return "inTransaction";
    return state === "idle" ? "idle" : "unknown";
  }
  if (state === "Sleep") return "idle";
  return state === "Daemon" || user === "event_scheduler" ? "background" : "running";
}

function createServer(driver: DriverKind, now: number): Server {
  const mysql = driver !== "postgres";
  const seeds = [...(mysql ? MYSQL_SEEDS : PG_SEEDS), ...poolSeeds(driver)];
  // 两份种子里第 5 个都是取快照用的那条连接。
  const selfId = seeds[4]![0];
  return {
    driver,
    startedAt: now - UPTIME_SECONDS * 1000,
    totals: {
      queries: 184_220_431,
      commits: 9_820_114,
      rollbacks: 12_406,
      received: 52_118_004_210,
      sent: 318_440_912_877,
      reads: 9_442_118_005,
      misses: 41_220_918,
    },
    slow: 37,
    takenAt: now,
    snapshots: 0,
    processes: seeds.map(([id, user, host, database, state, detail, ago, query]) => ({
      id,
      user,
      host,
      database,
      state,
      detail,
      since: now - ago * 1000,
      query,
      activity: activityOf(driver, state, user),
      // PostgreSQL 的 backend_start:比进入当前状态早一点。
      sessionStart: mysql ? null : new Date(now - ago * 1000 - 5_000).toISOString(),
      isSelf: id === selfId,
    })),
  };
}

/** 第 n 次快照时的每秒速率:几条不同周期的正弦叠起来,看着像真的负载。 */
function ratesAt(n: number) {
  const queries = 820 + 260 * Math.sin(n / 3) + 90 * Math.sin(n * 1.7);
  const missRatio = 0.004 + 0.003 * Math.sin(n / 5);
  return {
    queries,
    commits: queries * 0.18,
    rollbacks: 0.6 + 0.5 * Math.sin(n / 2),
    received: queries * 420,
    sent: queries * 2_650,
    reads: queries * 35,
    misses: queries * 35 * missRatio,
  };
}

function counters(server: Server, now: number): ServerCounters {
  const mysql = server.driver !== "postgres";
  const whole = (value: number) => Math.round(value);
  const t = server.totals;
  return {
    uptimeSeconds: Math.floor((now - server.startedAt) / 1000),
    connections: server.processes.length,
    maxConnections: mysql ? 151 : 100,
    running: server.processes.filter((p) => p.activity === "running").length,
    queries: mysql ? whole(t.queries) : null,
    commits: whole(t.commits),
    rollbacks: whole(t.rollbacks),
    bytesReceived: mysql ? whole(t.received) : null,
    bytesSent: mysql ? whole(t.sent) : null,
    cacheReads: whole(t.reads),
    cacheMisses: whole(t.misses),
    slowQueries: mysql ? server.slow : null,
  };
}

export function createMockMonitor() {
  const servers = new Map<string, Server>();
  const serverOf = (id: string, driver: DriverKind) => {
    if (driver === "sqlite") fail("unsupported", "SQLite 是 Basalt 自己打开的文件，没有服务器进程可看。");
    const existing = servers.get(id);
    if (existing) return existing;
    const created = createServer(driver, Date.now());
    servers.set(id, created);
    return created;
  };

  return {
    snapshot(id: string, driver: DriverKind): ServerSnapshot {
      const server = serverOf(id, driver);
      const now = Date.now();
      const seconds = Math.max(0, now - server.takenAt) / 1000;
      const rates = ratesAt(server.snapshots);
      for (const key of Object.keys(server.totals) as (keyof Server["totals"])[]) {
        server.totals[key] += rates[key] * seconds;
      }
      if (server.snapshots % 7 === 6) server.slow += 1;
      server.snapshots += 1;
      server.takenAt = now;
      return {
        counters: counters(server, now),
        // 取快照的那条连接正在执行的就是这次的查询,总是刚开始。
        processes: server.processes.map(({ since, ...process }) => ({
          ...process,
          seconds: process.isSelf ? 0 : Math.max(0, Math.floor((now - since) / 1000)),
        })),
        takenAtMs: now,
        limited: null,
      };
    },

    kill(id: string, driver: DriverKind, target: KillTarget): void {
      const server = serverOf(id, driver);
      const current = server.processes.find((p) => p.id === target.id);
      if (current?.isSelf) fail("invalid_data", "这是 Basalt 取监控数据用的连接，不能在这里终止。");
      if (!current || (target.sessionStart !== null && current.sessionStart !== target.sessionStart)) {
        fail("database", `会话 ${target.id} 已经结束了。`);
      }
      if (target.mode === "query") {
        if (current.activity !== "running" || (target.query !== null && current.query !== target.query)) {
          fail("database", `会话 ${target.id} 已经不在执行刚才那条语句了（执行完了，或者换了一条），没有终止。`);
        }
        server.processes = server.processes.map((p) => (p === current ? stopped(p, driver) : p));
      } else {
        server.processes = server.processes.filter((p) => p !== current);
      }
    },
  };
}

/** 语句被取消后的样子:MySQL 回到 Sleep,PostgreSQL 回到 idle 并留着上一条语句。 */
function stopped(process: MockProcess, driver: DriverKind): MockProcess {
  const postgres = driver === "postgres";
  return {
    ...process,
    state: postgres ? "idle" : "Sleep",
    detail: postgres ? "Client:ClientRead" : null,
    query: postgres ? process.query : null,
    activity: "idle",
    since: Date.now(),
  };
}
