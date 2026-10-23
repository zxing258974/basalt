import type { ReactNode } from "react";
import { formatBytes, formatCount } from "../../lib/format";
import type { RedisStats } from "../../lib/monitorTypes";
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

/** 连接数、内存到上限的这个比例就标红:再多就要被拒了(或开始淘汰键)。 */
const CONNECTIONS_WARNING = 0.9;
const MEMORY_WARNING = 0.9;

type RateKey = Exclude<keyof RatePoint, "at">;

function Tile({
  label,
  value,
  caption,
  wide,
  children,
}: {
  label: string;
  value: ReactNode;
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

/** 占上限的比例条:读数封顶在上限,原数写在 valuetext 里。 */
function Meter({
  label,
  value,
  max,
  crowded,
  text,
}: {
  label: string;
  value: number;
  max: number;
  crowded: boolean;
  text: string;
}) {
  return (
    <div
      className={crowded ? "metric-meter is-crowded" : "metric-meter"}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={Math.min(value, max)}
      aria-valuetext={text}
    >
      <span style={{ transform: `scaleX(${Math.min(1, value / max)})` }} />
    </div>
  );
}

function ConnectionsTile({
  connections,
  max,
  running,
  runningLabel,
}: {
  connections: number;
  max: number | null;
  running: number;
  runningLabel: string;
}) {
  const ratio = max ? Math.min(1, connections / max) : null;
  const crowded = ratio !== null && ratio >= CONNECTIONS_WARNING;
  return (
    <Tile
      label="连接"
      value={max ? `${formatCount(connections)} / ${formatCount(max)}` : formatCount(connections)}
      caption={
        <>
          {runningLabel} {formatCount(running)}
          {crowded && <span className="metric-alert"> · 接近上限</span>}
        </>
      }
    >
      {/* 超级用户的保留连接能让连接数超过上限。 */}
      {max !== null && (
        <Meter
          label="连接数占上限的比例"
          value={connections}
          max={max}
          crowded={crowded}
          text={`${formatCount(connections)} / ${formatCount(max)}`}
        />
      )}
    </Tile>
  );
}

/** Redis 用了多少内存;设了上限的话画比例条,快满了标红(满了就要淘汰键或拒绝写入)。 */
function MemoryTile({ stats }: { stats: RedisStats }) {
  const { memoryUsed: used, memoryMax: max } = stats;
  const crowded = used !== null && max !== null && used / max >= MEMORY_WARNING;
  const parts = [
    stats.memoryPeak !== null ? `峰值 ${formatBytes(stats.memoryPeak)}` : null,
    max === null ? "没设上限" : stats.evictionPolicy,
  ].filter(Boolean);
  return (
    <Tile
      label="内存"
      value={
        <>
          {formatBytes(used)}
          {max !== null && <span className="metric-value-of"> / {formatBytes(max)}</span>}
        </>
      }
      caption={
        <>
          {parts.join(" · ")}
          {crowded && <span className="metric-alert"> · 接近上限</span>}
        </>
      }
    >
      {used !== null && max !== null && (
        <Meter
          label="内存占上限的比例"
          value={used}
          max={max}
          crowded={crowded}
          text={`${formatBytes(used)} / ${formatBytes(max)}`}
        />
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
  const redis = driver === "redis" ? snapshot.redis : null;
  const rollbacks = history.points.at(-1)?.rollbacks ?? null;
  const slowGrowth = growthSince(history.first, snapshot, "slowQueries");

  return (
    <section className="metric-grid" aria-label="指标">
      {/* 标题层级:指标和下面的「会话」同级。 */}
      <h2 className="visually-hidden">指标</h2>
      {counters.queries !== null && (
        <RateTile
          history={history}
          rate="queries"
          label={redis ? "命令 / 秒" : "查询 / 秒"}
          format={formatPerSecond}
          wide
        />
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
      <ConnectionsTile
        connections={counters.connections}
        max={counters.maxConnections}
        running={counters.running}
        runningLabel={redis ? "阻塞" : "在执行"}
      />
      {redis && <MemoryTile stats={redis} />}
      {redis && (
        <Tile
          label="键"
          value={formatCount(redis.keys)}
          caption={[
            `${formatCount(redis.expires)} 个会过期`,
            redis.evictedKeys ? `淘汰过 ${formatCount(redis.evictedKeys)} 个` : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        />
      )}
      {counters.cacheReads !== null && (
        <RateTile
          history={history}
          rate="hitRate"
          label={redis ? "键命中率" : "缓存命中率"}
          format={formatPercent}
          zeroBased={false}
          fallback={lifetimeHitRate(counters)}
          caption={redis ? "读键时键存在的比例" : postgres ? "共享缓冲区" : "InnoDB 缓冲池"}
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
          label={redis ? "慢命令" : "慢查询"}
          value={formatCount(counters.slowQueries)}
          caption={
            redis ? "SLOWLOG 里现有的条数" : slowGrowth ? `打开监控以来 +${formatCount(slowGrowth)}` : "启动以来累计"
          }
        />
      )}
    </section>
  );
}
