import { useConnections, useServerInfo } from "../../hooks/useCatalog";
import { useUi } from "../../stores/ui";
import { DriverBadge } from "../connection/DriverBadge";
import { endpointSummary } from "../connection/driverMeta";
import { connectionIdFromKey } from "../sidebar/treeKeys";
import { NoticeArea } from "./NoticeArea";

export function StatusBar() {
  const connections = useConnections().data ?? [];
  const selectedId = useUi((s) => connectionIdFromKey(s.selectedKey));
  const connection = connections.find((c) => c.id === selectedId);
  const server = useServerInfo(connection?.isOpen ? connection.id : null).data;
  const openCount = connections.filter((c) => c.isOpen).length;

  return (
    <footer className="statusbar">
      <div className="statusbar-section">
        {connection ? (
          <>
            <DriverBadge driver={connection.driver} live={connection.isOpen} />
            <span className="statusbar-strong">{connection.name}</span>
            <span className="mono">{endpointSummary(connection)}</span>
            {server && (
              <span>
                {server.product} {server.version}
              </span>
            )}
            {!connection.isOpen && <span>未连接</span>}
          </>
        ) : (
          <span>未选择连接</span>
        )}
      </div>
      <div className="statusbar-section">
        <NoticeArea />
        <span>
          {connections.length} 个连接 · {openCount} 个已打开
        </span>
      </div>
    </footer>
  );
}
