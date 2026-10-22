import type { ReactNode } from "react";
import { formatCount } from "../../lib/format";
import type { DriverKind } from "../../lib/types";
import {
  formatBytesPerSecond,
  formatPerSecond,
  formatPercent,
  growthSince,
  lifetimeHitRate,
  type MonitorHistory,
  type RatePoint,
} from "./monitorModel";
import { Sparkline } from "./Sparkline";

/** 连接数到上限的这个比例就标红:再多就要被拒了。 */
const CONNECTIONS_WARNING = 0.9;

type RateKey = Exclude<keyof RatePoint, "at">;

function Tile({
  label,
  value,
  caption,
  wide,
  children,
}: {
  label: string;
  value: string;
  caption?: ReactNode;
  wide?: boolean;
  children?: ReactNode;
}) {
  return (
    <section className={wide ? "metric-tile is-wide" : "metric-tile"} aria-label={label}>
      <h3 className="metric-label">{label}</h3>
      <p className="metric-value mono">{value}</p>
      {caption && <p className="metric-caption">{caption}</p>}
      {children}
    </section>
  );
}

function RateTile({
  history,
  rate,
  label,
  format,
  caption,
  wide,
  zeroBased = true,
  fallback = null,
}: {
  history: MonitorHistory;
  rate: RateKey;
  label: string;
  format(value: number | null): string;
  caption?: ReactNode;
  wide?: boolean;
  zeroBased?: boolean;
  /** 还没有两份快照可比时显示的值。 */
  fallback?: number | null;
}) {
  const values = history.points.map((p) => p[rate]);
  const latest = history.points.at(-1)?.[rate] ?? fallback;
  return (
    <Tile label={label} value={format(latest)} caption={caption} wide={wide}>
      <Sparkline
        label={label}
        values={values}
        times={history.points.map((p) => p.at)}
        format={format}
        zeroBased={zeroBased}
      />
    </Tile>
  );
}

function ConnectionsTile({ connections, max, running }: { connections: number; max: number | null; running: number }) {
  const ratio = max ? Math.min(1, connections / max) : null;
  const crowded = ratio !== null && ratio >= CONNECTIONS_WARNING;
  return (
    <Tile
      label="连接"
      value={max ? `${formatCount(connections)} / ${formatCount(max)}` : formatCount(connections)}
      caption={
        <>
          在执行 {formatCount(running)}
          {crowded && <span className="metric-alert"> · 接近上限</span>}
        </>
      }
    >
      {ratio !== null && max !== null && (
        // 超级用户的保留连接能让连接数超过上限,读数封顶在上限,原数写在 valuetext 里。
        <div
          className={crowded ? "metric-meter is-crowded" : "metric-meter"}
          role="meter"
          aria-label="连接数占上限的比例"
          aria-valuemin={0}
          aria-valuemax={max}
          aria-valuenow={Math.min(connections, max)}
          aria-valuetext={`${formatCount(connections)} / ${formatCount(max)}`}
        >
          <span style={{ transform: `scaleX(${ratio})` }} />
        </div>
      )}
    </Tile>
  );
}

/** 最上面一排指标:这种数据库读不到的项不显示。 */
export function MetricTiles({ history, driver }: { history: MonitorHistory; driver: DriverKind }) {
  const snapshot = history.last;
  if (!snapshot) return null;
  const { counters } = snapshot;
  const postgres = driver === "postgres";
  const rollbacks = history.points.at(-1)?.rollbacks ?? null;
  const slowGrowth = growthSince(history.first, snapshot, "slowQueries");

  return (
    <section className="metric-grid" aria-label="指标">
      {/* 标题层级:指标和下面的「会话」同级。 */}
      <h2 className="visually-hidden">指标</h2>
      {counters.queries !== null && (
        <RateTile history={history} rate="queries" label="查询 / 秒" format={formatPerSecond} wide />
      )}
      {counters.commits !== null && (
        <RateTile
          history={history}
          rate="commits"
          label={postgres ? "事务提交 / 秒" : "COMMIT / 秒"}
          format={formatPerSecond}
          wide={counters.queries === null}
          caption={`回滚 ${formatPerSecond(rollbacks)} / 秒`}
        />
      )}
      <ConnectionsTile connections={counters.connections} max={counters.maxConnections} running={counters.running} />
      {counters.cacheReads !== null && (
        <RateTile
          history={history}
          rate="hitRate"
          label="缓存命中率"
          format={formatPercent}
          zeroBased={false}
          fallback={lifetimeHitRate(counters)}
          caption={postgres ? "共享缓冲区" : "InnoDB 缓冲池"}
        />
      )}
      {counters.bytesReceived !== null && (
        <RateTile history={history} rate="received" label="接收" format={formatBytesPerSecond} />
      )}
      {counters.bytesSent !== null && (
        <RateTile history={history} rate="sent" label="发送" format={formatBytesPerSecond} />
      )}
      {counters.slowQueries !== null && (
        <Tile
          label="慢查询"
          value={formatCount(counters.slowQueries)}
          caption={slowGrowth ? `打开监控以来 +${formatCount(slowGrowth)}` : "启动以来累计"}
        />
      )}
    </section>
  );
}
