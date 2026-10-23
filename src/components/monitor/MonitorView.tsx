import { Pause, Play, RefreshCw, Search } from "lucide-react";
import { useMemo, useState } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import type { KillMode, ServerProcess, ServerSnapshot } from "../../lib/monitorTypes";
import { errorMessage } from "../../lib/types";
import type { MonitorTab } from "../../stores/tabs";
import { DriverBadge } from "../connection/DriverBadge";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import { Checkbox } from "../ui/Field";
import { KillProcessDialog } from "./KillProcessDialog";
import { MetricTiles } from "./MetricTiles";
import { monitorWords } from "./monitorWords";
import { formatClock, formatDuration, runningCount, visibleProcesses } from "./monitorModel";
import { ProcessTable } from "./ProcessTable";
import { SlowLog } from "./SlowLog";
import { useServerMonitor } from "./useServerMonitor";

const INTERVALS = [
  { ms: 2_000, label: "2 秒" },
  { ms: 5_000, label: "5 秒" },
  { ms: 10_000, label: "10 秒" },
] as const;

const ROLES: Record<string, string> = { master: "主节点", slave: "从节点" };

/** 标题栏右边:Redis 是主是从、服务器跑了多久、数据是几点的。 */
function summary(snapshot: ServerSnapshot, paused: boolean): string {
  const { counters, takenAtMs } = snapshot;
  const role = snapshot.redis?.role;
  const parts = [
    role ? (ROLES[role] ?? role) : null,
    counters.uptimeSeconds === null ? null : `已运行 ${formatDuration(counters.uptimeSeconds)}`,
    `${paused ? "已暂停，停在" : "更新于"} ${formatClock(takenAtMs)}`,
  ];
  return parts.filter(Boolean).join(" · ");
}

interface Killing {
  process: ServerProcess;
  mode: KillMode;
}

/** 服务器监控:最上面一排指标和走势,下面是会话列表。标签页切走、点了暂停就不再去查。 */
export function MonitorView({ tab, active }: { tab: MonitorTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const { openWithFeedback } = useConnectionActions();
  const [intervalMs, setIntervalMs] = useState<number>(INTERVALS[0].ms);
  const [paused, setPaused] = useState(false);
  const [hideIdle, setHideIdle] = useState(false);
  const [search, setSearch] = useState("");
  const [killing, setKilling] = useState<Killing | null>(null);
  // 只有手动刷新时转圈:后台轮询也转的话,焦点在按钮上的人每隔两秒就丢一次焦点。
  const [refreshing, setRefreshing] = useState(false);
  const isOpen = Boolean(connection?.isOpen);
  const words = monitorWords(connection?.driver);
  const { query, history } = useServerMonitor({
    connectionId: tab.connectionId,
    enabled: isOpen,
    intervalMs: active && !paused ? intervalMs : null,
  });

  const snapshot = history.last;
  const processes = useMemo(
    () => (snapshot ? visibleProcesses(snapshot.processes, { hideIdle, search }) : []),
    [snapshot, hideIdle, search],
  );
  const total = snapshot?.processes.length ?? 0;
  const running = snapshot ? runningCount(snapshot.processes) : 0;
  const refresh = () => {
    setRefreshing(true);
    void query.refetch().finally(() => setRefreshing(false));
  };

  return (
    <section className="monitor" aria-label={`${connection?.name ?? ""} 的服务器监控`}>
      <header className="objects-header">
        <div className="objects-heading">
          {connection && <DriverBadge driver={connection.driver} />}
          <span className="objects-crumb">{connection?.name ?? "已关闭的连接"}</span>
          <span className="objects-crumb-sep">/</span>
          <h1 className="objects-title">{tab.title}</h1>
          <span className="objects-summary">{snapshot ? summary(snapshot, paused) : ""}</span>
          {/* 读屏只念暂停、继续,不每隔两秒念一遍时间。 */}
          <span className="visually-hidden" aria-live="polite">
            {paused ? "已暂停刷新" : ""}
          </span>
        </div>
        <div className="objects-toolbar">
          <div className="segmented" role="radiogroup" aria-label="刷新间隔">
            {INTERVALS.map((option) => (
              <button
                key={option.ms}
                role="radio"
                aria-checked={intervalMs === option.ms}
                className={intervalMs === option.ms ? "segmented-item is-active" : "segmented-item"}
                onClick={() => setIntervalMs(option.ms)}
              >
                {option.label}
              </button>
            ))}
          </div>
          <Button
            variant="ghost"
            size="sm"
            icon={paused ? <Play /> : <Pause />}
            aria-pressed={paused}
            onClick={() => setPaused((p) => !p)}
          >
            {paused ? "继续" : "暂停"}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<RefreshCw className={refreshing ? "monitor-refresh is-spinning" : "monitor-refresh"} />}
            aria-label="立即刷新"
            disabled={!isOpen}
            onClick={refresh}
          />
        </div>
      </header>

      {!connection?.isOpen ? (
        <EmptyState
          title={connection ? "连接已关闭" : "这个连接已经不存在"}
          action={
            connection && (
              <Button variant="primary" onClick={() => void openWithFeedback(connection)}>
                重新打开
              </Button>
            )
          }
        />
      ) : !snapshot && query.error ? (
        <EmptyState
          title="读取服务器状态失败"
          detail={errorMessage(query.error)}
          tone="error"
          action={<Button onClick={() => void query.refetch()}>重试</Button>}
        />
      ) : !snapshot ? (
        <EmptyState title="正在读取服务器状态…" />
      ) : (
        <div className="monitor-scroll">
          {query.error && (
            <p className="monitor-banner is-error" role="alert">
              刷新失败：{errorMessage(query.error)}。下面是 {formatClock(snapshot.takenAtMs)} 的数据。
            </p>
          )}
          <MetricTiles history={history} driver={connection.driver} />
          <section className="monitor-sessions" aria-labelledby={`${tab.id}-sessions`}>
            <header className="monitor-sessions-header">
              <h2 id={`${tab.id}-sessions`} className="monitor-sessions-title">
                会话
              </h2>
              <span className="monitor-sessions-count">
                共 {total} 个 · {words.running} {running}
              </span>
              <Checkbox label="隐藏空闲" checked={hideIdle} onCheckedChange={setHideIdle} />
              <label className="search-box">
                <Search aria-hidden />
                <input
                  value={search}
                  placeholder={words.search}
                  aria-label="筛选会话"
                  onChange={(e) => setSearch(e.target.value)}
                  spellCheck={false}
                />
              </label>
            </header>
            {snapshot.limited && <p className="monitor-banner">{snapshot.limited}</p>}
            {processes.length === 0 ? (
              <p className="monitor-sessions-empty">{total === 0 ? "没有会话" : "没有符合筛选条件的会话"}</p>
            ) : (
              <ProcessTable
                processes={processes}
                words={words}
                onKill={(process, mode) => setKilling({ process, mode })}
              />
            )}
          </section>
          {snapshot.redis && <SlowLog entries={snapshot.redis.slowlog} headingId={`${tab.id}-slowlog`} />}
        </div>
      )}
      {killing && (
        <KillProcessDialog
          connectionId={tab.connectionId}
          process={killing.process}
          mode={killing.mode}
          words={words}
          onClose={() => setKilling(null)}
          onDone={refresh}
        />
      )}
    </section>
  );
}
