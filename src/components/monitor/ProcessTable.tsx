import { CircleStop, Unplug } from "lucide-react";
import type { KillMode, ServerProcess } from "../../lib/monitorTypes";
import { formatDuration } from "./monitorModel";

const SELF_HINT = "Basalt 取监控数据用的就是这条连接";
const BACKGROUND_HINT = "服务器自己的线程（事件调度器、复制）";
const UNKNOWN_HINT = "账号权限不够，看不到它在做什么";

interface ProcessTableProps {
  processes: readonly ServerProcess[];
  onKill(process: ServerProcess, mode: KillMode): void;
}

export function ProcessTable({ processes, onKill }: ProcessTableProps) {
  return (
    <table className="process-table">
      <thead>
        <tr>
          <th className="is-numeric">编号</th>
          <th>用户</th>
          <th className="col-host">客户端</th>
          <th className="col-database">数据库</th>
          <th>状态</th>
          <th className="is-numeric">时长</th>
          <th className="col-query">语句</th>
          <th className="col-actions">
            <span className="visually-hidden">操作</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {processes.map((process) => (
          <ProcessRow key={process.id} process={process} onKill={onKill} />
        ))}
      </tbody>
    </table>
  );
}

/** 按钮为什么不能点;能点时为 null。 */
function queryBlocker(process: ServerProcess): string | null {
  if (process.isSelf) return SELF_HINT;
  switch (process.activity) {
    case "running":
      return null;
    case "background":
      return BACKGROUND_HINT;
    case "unknown":
      return UNKNOWN_HINT;
    default:
      return "没有在执行的语句";
  }
}

function connectionBlocker(process: ServerProcess): string | null {
  if (process.isSelf) return SELF_HINT;
  return process.activity === "background" ? BACKGROUND_HINT : null;
}

function ProcessRow({ process, onKill }: { process: ServerProcess; onKill: ProcessTableProps["onKill"] }) {
  const quiet = process.activity === "idle" || process.activity === "background";
  const classes = [quiet && "is-idle", process.isSelf && "is-self"].filter(Boolean).join(" ");
  const stopQuery = queryBlocker(process);
  const disconnect = connectionBlocker(process);
  return (
    <tr className={classes || undefined}>
      <td className="is-numeric mono">
        {process.isSelf && (
          <span className="process-self" title={SELF_HINT}>
            Basalt
          </span>
        )}
        <span className="selectable">{process.id}</span>
      </td>
      <td>{process.user ?? ""}</td>
      <td className="col-host mono">{process.host ?? ""}</td>
      <td className="col-database">{process.database ?? ""}</td>
      <td>
        {process.activity === "unknown" && process.state === null ? (
          <span className="process-unknown" title={UNKNOWN_HINT}>
            看不到
          </span>
        ) : (
          <span className="process-state">
            {process.activity === "running" && <span className="process-pulse" aria-hidden />}
            {process.state ?? ""}
          </span>
        )}
        {process.detail && (
          <span className="process-detail" title={process.detail}>
            {process.detail}
          </span>
        )}
      </td>
      <td className="is-numeric mono">{formatDuration(process.seconds)}</td>
      <td className="col-query mono" title={process.query ?? undefined}>
        <span className="selectable">{process.query ?? ""}</span>
      </td>
      <td className="col-actions">
        <button
          className="process-action"
          aria-label={`终止会话 ${process.id} 正在执行的语句`}
          title={stopQuery ?? "终止查询"}
          disabled={stopQuery !== null}
          onClick={() => onKill(process, "query")}
        >
          <CircleStop />
        </button>
        <button
          className="process-action is-danger"
          aria-label={`断开会话 ${process.id}`}
          title={disconnect ?? "断开连接"}
          disabled={disconnect !== null}
          onClick={() => onKill(process, "connection")}
        >
          <Unplug />
        </button>
      </td>
    </tr>
  );
}
