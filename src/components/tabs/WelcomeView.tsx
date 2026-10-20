import { ArrowRight, Plus } from "lucide-react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { useUi } from "../../stores/ui";
import { DriverBadge } from "../connection/DriverBadge";
import { endpointSummary } from "../connection/driverMeta";
import { connectionKey } from "../sidebar/treeKeys";
import { Button } from "../ui/Button";

const SHORTCUTS = [
  ["⌘N", "新建连接"],
  ["⌘W", "关闭标签页"],
  ["↑ ↓ ← →", "在对象树里移动、展开、收起"],
  ["↩", "打开选中的库或 schema"],
] as const;

export function WelcomeView() {
  const connections = useConnections().data ?? [];
  const actions = useConnectionActions();
  const { openConnectionDialog, select } = useUi.getState();

  return (
    <div className="welcome">
      <section className="welcome-hero">
        <p className="welcome-kicker">Basalt</p>
        <h1 className="welcome-title">
          连上库，
          <br />
          看清每一张表。
        </h1>
        <p className="welcome-lede">
          MySQL、MariaDB、PostgreSQL、SQLite。在左侧展开连接，双击库或 schema 打开它的对象列表。
        </p>
        <Button
          variant="primary"
          icon={<Plus />}
          onClick={() => openConnectionDialog({ mode: "create", driver: null })}
        >
          新建连接
        </Button>
        <dl className="welcome-shortcuts">
          {SHORTCUTS.map(([keys, label]) => (
            <div key={keys} className="welcome-shortcut">
              <dt>
                <kbd>{keys}</kbd>
              </dt>
              <dd>{label}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="welcome-connections" aria-label="连接">
        <h2 className="welcome-section-title">连接</h2>
        {connections.length === 0 ? (
          <p className="welcome-empty">还没有连接。先新建一个，配置只保存在本机，密码进钥匙串。</p>
        ) : (
          <ul className="welcome-list">
            {connections.map((connection) => (
              <li key={connection.id}>
                <button
                  className="welcome-connection"
                  onClick={() => {
                    select(connectionKey(connection.id));
                    void actions.openWithFeedback(connection);
                  }}
                >
                  <DriverBadge driver={connection.driver} size="lg" live={connection.isOpen} />
                  <span className="welcome-connection-text">
                    <span className="welcome-connection-name">{connection.name}</span>
                    <span className="welcome-connection-endpoint mono">{endpointSummary(connection)}</span>
                  </span>
                  <ArrowRight className="welcome-connection-arrow" aria-hidden />
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
