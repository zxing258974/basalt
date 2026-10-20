import { TableProperties, X } from "lucide-react";
import { useConnections } from "../../hooks/useCatalog";
import { useTabs } from "../../stores/tabs";
import { driverMeta } from "../connection/driverMeta";

export function TabBar() {
  const { tabs, activeId, activate, close } = useTabs();
  const connections = useConnections().data ?? [];
  if (tabs.length === 0) return null;

  return (
    <div className="tabbar" role="tablist" aria-label="打开的标签页">
      {tabs.map((tab) => {
        const connection = connections.find((c) => c.id === tab.connectionId);
        const color = connection ? driverMeta(connection.driver).color : "var(--line-strong)";
        const active = tab.id === activeId;
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            className={active ? "tab is-active" : "tab"}
            style={{ "--tab-color": color } as React.CSSProperties}
            title={connection ? `${connection.name} / ${tab.title}` : tab.title}
            onClick={() => activate(tab.id)}
            onAuxClick={(event) => event.button === 1 && close(tab.id)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") activate(tab.id);
            }}
          >
            <TableProperties className="tab-icon" aria-hidden />
            <span className="tab-title">{tab.title}</span>
            <button
              className="tab-close"
              aria-label={`关闭 ${tab.title}`}
              tabIndex={-1}
              onClick={(event) => {
                event.stopPropagation();
                close(tab.id);
              }}
            >
              <X />
            </button>
          </div>
        );
      })}
    </div>
  );
}
