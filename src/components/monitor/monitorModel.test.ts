import { describe, expect, it } from "vitest";
import type { ServerCounters, ServerProcess, ServerSnapshot } from "../../lib/monitorTypes";
import {
  EMPTY_HISTORY,
  formatBytesPerSecond,
  formatDuration,
  formatPerSecond,
  formatPercent,
  growthSince,
  lifetimeHitRate,
  ratesBetween,
  recordSnapshot,
  runningCount,
  visibleProcesses,
} from "./monitorModel";

const COUNTERS: ServerCounters = {
  uptimeSeconds: 1000,
  connections: 10,
  maxConnections: 151,
  running: 2,
  queries: 10_000,
  commits: 500,
  rollbacks: 3,
  bytesReceived: 1_000_000,
  bytesSent: 8_000_000,
  cacheReads: 100_000,
  cacheMisses: 1_000,
  slowQueries: 4,
};

function snapshot(at: number, counters: Partial<ServerCounters> = {}): ServerSnapshot {
  return { counters: { ...COUNTERS, ...counters }, processes: [], takenAtMs: at, limited: null };
}

function process(id: number, partial: Partial<ServerProcess> = {}): ServerProcess {
  return {
    id,
    user: "app",
    host: "10.0.0.1:4000",
    database: "shop",
    state: "Query",
    detail: null,
    seconds: 1,
    query: "SELECT 1",
    activity: "running",
    sessionStart: null,
    isSelf: false,
    ...partial,
  };
}

describe("rates between two snapshots", () => {
  it("divides each counter's growth by the seconds between them", () => {
    const point = ratesBetween(
      snapshot(10_000),
      snapshot(12_000, {
        queries: 12_000,
        commits: 600,
        bytesReceived: 1_200_000,
        cacheReads: 110_000,
        cacheMisses: 1_100,
      }),
    );
    expect(point).toEqual({
      at: 12_000,
      queries: 1000,
      commits: 50,
      rollbacks: 0,
      received: 100_000,
      sent: 0,
      hitRate: 0.99,
    });
  });

  it("leaves out counters that are missing or went backwards after a restart", () => {
    const point = ratesBetween(snapshot(0, { queries: null }), snapshot(1000, { queries: 5, commits: 10 }));
    expect(point?.queries).toBeNull();
    expect(point?.commits).toBeNull();
  });

  it("has no hit rate when nothing was read", () => {
    expect(ratesBetween(snapshot(0), snapshot(1000))?.hitRate).toBeNull();
  });

  it("refuses snapshots taken at the same moment or out of order", () => {
    expect(ratesBetween(snapshot(1000), snapshot(1000))).toBeNull();
    expect(ratesBetween(snapshot(2000), snapshot(1000))).toBeNull();
  });
});

describe("history", () => {
  it("starts collecting points from the second snapshot", () => {
    const first = recordSnapshot(EMPTY_HISTORY, snapshot(0));
    expect(first.points).toEqual([]);
    expect(first.first).toBe(first.last);
    const second = recordSnapshot(first, snapshot(2000, { queries: 12_000 }));
    expect(second.points.map((p) => p.queries)).toEqual([1000]);
    expect(second.first?.takenAtMs).toBe(0);
    expect(second.last?.takenAtMs).toBe(2000);
  });

  it("takes a snapshot it cannot compare as the newest without adding a point", () => {
    const once = recordSnapshot(recordSnapshot(EMPTY_HISTORY, snapshot(0)), snapshot(1000));
    // 本机时间往回拨了:这一次不添点,下一次照常接着算。
    const stepped = recordSnapshot(once, snapshot(500, { queries: 10_500 }));
    expect(stepped.points).toHaveLength(1);
    expect(stepped.last?.takenAtMs).toBe(500);
    expect(recordSnapshot(stepped, snapshot(1500, { queries: 11_500 })).points.at(-1)?.queries).toBe(1000);
  });

  it("starts over after the server restarted", () => {
    const before = recordSnapshot(recordSnapshot(EMPTY_HISTORY, snapshot(0)), snapshot(1000, { slowQueries: 9 }));
    // 重启之后计数器从头数:有的变小了,有的碰巧还变大了,都不能拿来算速率。
    const after = recordSnapshot(before, snapshot(2000, { uptimeSeconds: 5, queries: 20_000, slowQueries: 1 }));
    expect(after.points).toHaveLength(1);
    expect(after.first?.takenAtMs).toBe(2000);
    const next = recordSnapshot(after, snapshot(3000, { uptimeSeconds: 6, queries: 20_100, slowQueries: 2 }));
    expect(next.points.at(-1)?.queries).toBe(100);
    expect(growthSince(next.first, next.last!, "slowQueries")).toBe(1);
  });

  it("keeps only the newest points", () => {
    let history = EMPTY_HISTORY;
    for (let i = 0; i < 6; i += 1) history = recordSnapshot(history, snapshot(i * 1000, { queries: i * 10 }), 3);
    expect(history.points.map((p) => p.at)).toEqual([3000, 4000, 5000]);
  });
});

describe("lifetime figures", () => {
  it("works out the hit rate since the server started", () => {
    expect(lifetimeHitRate(COUNTERS)).toBeCloseTo(0.99);
    expect(lifetimeHitRate({ ...COUNTERS, cacheReads: 0 })).toBeNull();
    expect(lifetimeHitRate({ ...COUNTERS, cacheMisses: null })).toBeNull();
  });

  it("counts growth since monitoring started", () => {
    expect(growthSince(snapshot(0), snapshot(1000, { slowQueries: 7 }), "slowQueries")).toBe(3);
    expect(growthSince(null, snapshot(1000), "slowQueries")).toBeNull();
    expect(growthSince(snapshot(0), snapshot(1000, { slowQueries: 1 }), "slowQueries")).toBeNull();
  });
});

describe("formatting", () => {
  it("formats rates, bytes and percentages", () => {
    expect(formatPerSecond(null)).toBe("—");
    expect(formatPerSecond(3.14)).toBe("3.1");
    expect(formatPerSecond(1234.6)).toBe("1,235");
    expect(formatBytesPerSecond(2048)).toBe("2 KB/s");
    expect(formatBytesPerSecond(null)).toBe("—");
    expect(formatPercent(0.99613)).toBe("99.61%");
    expect(formatPercent(null)).toBe("—");
  });

  it("shows durations in their two largest units", () => {
    expect(formatDuration(0)).toBe("0 秒");
    expect(formatDuration(8)).toBe("8 秒");
    expect(formatDuration(312)).toBe("5 分 12 秒");
    expect(formatDuration(3600)).toBe("1 小时");
    expect(formatDuration(3 * 86_400 + 4 * 3600 + 59)).toBe("3 天 4 小时");
    expect(formatDuration(null)).toBe("—");
    expect(formatDuration(-1)).toBe("—");
  });
});

describe("session list", () => {
  const sessions = [
    process(1, { activity: "idle", state: "Sleep", seconds: 900, query: null }),
    process(2, { seconds: 3 }),
    process(3, { seconds: 40, user: "report", query: "SELECT * FROM orders" }),
    process(4, { seconds: null }),
  ];
  const ids = (list: readonly ServerProcess[]) => list.map((p) => p.id);

  it("puts running sessions first, longest running first", () => {
    expect(ids(visibleProcesses(sessions, { hideIdle: false, search: "" }))).toEqual([3, 2, 4, 1]);
    expect(runningCount(sessions)).toBe(3);
  });

  it("orders open transactions and hidden sessions before idle ones and server threads last", () => {
    const mixed = [
      process(10, { activity: "background", state: "Daemon", seconds: 90_000 }),
      process(11, { activity: "idle", seconds: 5 }),
      process(12, { activity: "unknown", state: null, seconds: null }),
      process(13, { activity: "inTransaction", state: "idle in transaction", seconds: 70 }),
      process(14, { seconds: 2 }),
    ];
    expect(ids(visibleProcesses(mixed, { hideIdle: false, search: "" }))).toEqual([14, 13, 12, 11, 10]);
    expect(ids(visibleProcesses(mixed, { hideIdle: true, search: "" }))).toEqual([14, 13, 12]);
    expect(runningCount(mixed)).toBe(1);
  });

  it("hides idle sessions and filters on any field without touching the input", () => {
    expect(visibleProcesses(sessions, { hideIdle: true, search: "" }).map((p) => p.id)).toEqual([3, 2, 4]);
    expect(visibleProcesses(sessions, { hideIdle: false, search: " ORDERS " }).map((p) => p.id)).toEqual([3]);
    expect(visibleProcesses(sessions, { hideIdle: false, search: "sleep" }).map((p) => p.id)).toEqual([1]);
    expect(visibleProcesses(sessions, { hideIdle: false, search: "report" }).map((p) => p.id)).toEqual([3]);
    expect(visibleProcesses(sessions, { hideIdle: true, search: "10.0.0.1" }).map((p) => p.id)).toEqual([3, 2, 4]);
    expect(sessions.map((p) => p.id)).toEqual([1, 2, 3, 4]);
  });
});
