import { describe, expect, it, vi } from "vitest";
import type { KillMode, KillTarget } from "../monitorTypes";
import type { CommandError } from "../types";
import { createMockMonitor } from "./monitorMock";

function thrown(action: () => void): CommandError {
  try {
    action();
  } catch (error) {
    return error as CommandError;
  }
  throw new Error("没有报错");
}

describe("mock server monitor", () => {
  it("counts up between snapshots and ages the sessions", () => {
    vi.useFakeTimers({ now: 1_000_000 });
    try {
      const monitor = createMockMonitor();
      const first = monitor.snapshot("c", "mysql");
      vi.advanceTimersByTime(2000);
      const second = monitor.snapshot("c", "mysql");
      expect(second.takenAtMs - first.takenAtMs).toBe(2000);
      expect(second.counters.queries!).toBeGreaterThan(first.counters.queries!);
      expect(second.counters.uptimeSeconds! - first.counters.uptimeSeconds!).toBe(2);
      const aged = second.processes.find((p) => p.id === 1184)!;
      expect(aged.seconds).toBe(first.processes.find((p) => p.id === 1184)!.seconds! + 2);
      expect(second.processes.filter((p) => p.isSelf).map((p) => p.id)).toEqual([1210]);
      expect(second.counters.connections).toBe(second.processes.length);
    } finally {
      vi.useRealTimers();
    }
  });

  it("has no query or network counters for PostgreSQL", () => {
    const { counters } = createMockMonitor().snapshot("c", "postgres");
    expect(counters.queries).toBeNull();
    expect(counters.bytesSent).toBeNull();
    expect(counters.commits).not.toBeNull();
  });

  const byId = (id: number, mode: KillMode): KillTarget => ({ id, mode, sessionStart: null, query: null });

  it("stops a statement or drops the whole session", () => {
    const monitor = createMockMonitor();
    const seen = monitor.snapshot("c", "mysql").processes.find((p) => p.id === 1184)!;
    expect(seen.activity).toBe("running");
    monitor.kill("c", "mysql", { id: 1184, mode: "query", sessionStart: null, query: seen.query });
    const stopped = monitor.snapshot("c", "mysql").processes.find((p) => p.id === 1184);
    expect(stopped).toMatchObject({ state: "Sleep", activity: "idle", query: null, seconds: 0 });
    monitor.kill("c", "mysql", byId(1184, "connection"));
    expect(monitor.snapshot("c", "mysql").processes.some((p) => p.id === 1184)).toBe(false);
  });

  it("refuses when the session is not what was confirmed", () => {
    const monitor = createMockMonitor();
    const seen = monitor.snapshot("p", "postgres").processes.find((p) => p.id === 41822)!;
    expect(seen.sessionStart).not.toBeNull();
    const moved = { id: 41822, mode: "query" as const, sessionStart: seen.sessionStart, query: "SELECT 2" };
    expect(thrown(() => monitor.kill("p", "postgres", moved)).message).toContain("不在执行刚才那条语句");
    const reused = { ...byId(41822, "connection"), sessionStart: "2000-01-01T00:00:00.000Z" };
    expect(thrown(() => monitor.kill("p", "postgres", reused)).message).toBe("会话 41822 已经结束了。");
    expect(thrown(() => monitor.kill("c", "mysql", byId(99, "connection"))).message).toBe("会话 99 已经结束了。");
    expect(thrown(() => monitor.kill("c", "mysql", byId(1210, "connection"))).code).toBe("invalid_data");
  });

  it("tells server threads and open transactions apart", () => {
    const mysql = createMockMonitor().snapshot("c", "mysql").processes;
    expect(mysql.find((p) => p.id === 5)?.activity).toBe("background");
    const postgres = createMockMonitor().snapshot("p", "postgres").processes;
    expect(postgres.find((p) => p.id === 41851)?.activity).toBe("inTransaction");
  });

  it("has nothing to show for SQLite", () => {
    expect(thrown(() => createMockMonitor().snapshot("s", "sqlite")).code).toBe("unsupported");
  });
});
