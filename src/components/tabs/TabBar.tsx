import { PencilRuler, Table2, TableProperties, X } from "lucide-react";
import { useConnections } from "../../hooks/useCatalog";
import { closeTabGuarded } from "../../stores/discard";
import { useTabs, type Tab } from "../../stores/tabs";
import { driverMeta } from "../connection/driverMeta";

export function TabBar() {
  const { tabs, activeId, activate } = useTabs();
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
            title={tabTooltip(tab, connection?.name)}
            onClick={() => activate(tab.id)}
            onAuxClick={(event) => event.button === 1 && closeTabGuarded(tab.id)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") activate(tab.id);
            }}
          >
            {tab.kind === "tableData" ? (
              <Table2 className="tab-icon" aria-hidden />
            ) : tab.kind === "tableDesign" ? (
              <PencilRuler className="tab-icon" aria-hidden />
            ) : (
              <TableProperties className="tab-icon" aria-hidden />
            )}
            <span className="tab-title">{tab.title}</span>
            <button
              className="tab-close"
              aria-label={`关闭 ${tab.title}`}
              tabIndex={-1}
              onClick={(event) => {
                event.stopPropagation();
                closeTabGuarded(tab.id);
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

function tabTooltip(tab: Tab, connectionName: string | undefined): string {
  const path =
    tab.kind === "tableData"
      ? [tab.table.database, tab.table.schema, tab.table.table].filter(Boolean).join(".")
      : tab.kind === "tableDesign"
        ? `${[tab.database, tab.schema, tab.table ?? "新表"].filter(Boolean).join(".")}（表结构）`
        : tab.title;
  return connectionName ? `${connectionName} / ${path}` : path;
}
