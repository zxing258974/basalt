import type { SlowCommand } from "../../lib/monitorTypes";
import { formatClock } from "./monitorModel";

/** 今天的只写时分秒,更早的带上月日。 */
export function formatWhen(unixSeconds: number, now = Date.now()): string {
  const at = new Date(unixSeconds * 1000);
  const today = new Date(now).toDateString() === at.toDateString();
  if (today) return formatClock(at.getTime());
  const day = `${String(at.getMonth() + 1).padStart(2, "0")}-${String(at.getDate()).padStart(2, "0")}`;
  return `${day} ${formatClock(at.getTime())}`;
}

/** 微秒写成人看的:不到 1 毫秒写微秒,其余写毫秒(两位小数)。 */
export function formatMicros(micros: number): string {
  return micros < 1000 ? `${micros} µs` : `${(micros / 1000).toFixed(micros < 100_000 ? 2 : 0)} ms`;
}

/** Redis 的慢日志:比 slowlog-log-slower-than 慢的命令,新的在前。 */
export function SlowLog({ entries, headingId }: { entries: readonly SlowCommand[] | null; headingId: string }) {
  return (
    <section className="monitor-sessions slowlog" aria-labelledby={headingId}>
      <header className="monitor-sessions-header">
        <h2 id={headingId} className="monitor-sessions-title">
          慢日志
        </h2>
        <span className="monitor-sessions-count">
          {entries === null ? "" : `最近 ${entries.length} 条 · 超过 slowlog-log-slower-than 的命令`}
        </span>
      </header>
      {entries === null ? (
        <p className="monitor-sessions-empty">读不到慢日志：这个账号不能执行 SLOWLOG，或者服务器禁用了它。</p>
      ) : entries.length === 0 ? (
        <p className="monitor-sessions-empty">没有慢命令</p>
      ) : (
        <table className="process-table slowlog-table">
          <thead>
            <tr>
              <th className="is-numeric">编号</th>
              <th>时间</th>
              <th className="is-numeric">耗时</th>
              <th className="col-query">命令</th>
              <th className="col-host">客户端</th>
            </tr>
          </thead>
          <tbody>
            {entries.map((entry) => (
              <tr key={entry.id}>
                <td className="is-numeric mono">{entry.id}</td>
                <td className="mono" title={new Date(entry.at * 1000).toLocaleString()}>
                  {formatWhen(entry.at)}
                </td>
                <td className="is-numeric mono">{formatMicros(entry.micros)}</td>
                <td className="col-query mono" title={entry.command}>
                  <span className="selectable">{entry.command}</span>
                </td>
                <td className="col-host mono">
                  {entry.client ?? ""}
                  {entry.name && <span className="process-detail">{entry.name}</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
