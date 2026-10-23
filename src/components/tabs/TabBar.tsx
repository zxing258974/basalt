import { Activity, FileCode2, KeyRound, PencilRuler, SquareTerminal, Table2, TableProperties, X } from "lucide-react";
import { useConnections } from "../../hooks/useCatalog";
import { closeTabGuarded } from "../../stores/discard";
import { useTabs, type Tab } from "../../stores/tabs";
import { driverMeta } from "../connection/driverMeta";

export function TabBar() {
  const { tabs, activeId, activate, unsaved } = useTabs();
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
            <TabIcon tab={tab} />
            <span className="tab-title">{tab.title}</span>
            {unsaved[tab.id] && (
              <span className="tab-dirty" title="有没保存的改动">
                <span className="visually-hidden">（未保存）</span>
              </span>
            )}
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

function TabIcon({ tab }: { tab: Tab }) {
  switch (tab.kind) {
    case "tableData":
      return <Table2 className="tab-icon" aria-hidden />;
    case "query":
      return <FileCode2 className="tab-icon" aria-hidden />;
    case "tableDesign":
      return <PencilRuler className="tab-icon" aria-hidden />;
    case "objects":
      return <TableProperties className="tab-icon" aria-hidden />;
    case "monitor":
      return <Activity className="tab-icon" aria-hidden />;
    case "redisKeys":
      return <KeyRound className="tab-icon" aria-hidden />;
    case "redisConsole":
      return <SquareTerminal className="tab-icon" aria-hidden />;
  }
}

function tabPath(tab: Tab): string {
  switch (tab.kind) {
    case "tableData":
      return [tab.table.database, tab.table.schema, tab.table.table].filter(Boolean).join(".");
    case "query": {
      const where = [tab.scope.database, tab.scope.schema].filter(Boolean).join(".");
      return where ? `${where} / ${tab.title}` : tab.title;
    }
    case "tableDesign":
      return `${[tab.database, tab.schema, tab.table ?? "新表"].filter(Boolean).join(".")}（表结构）`;
    case "objects":
    case "monitor":
    case "redisKeys":
      return tab.title;
    case "redisConsole":
      return `${tab.title}（db${tab.db}）`;
  }
}

function tabTooltip(tab: Tab, connectionName: string | undefined): string {
  const path = tabPath(tab);
  return connectionName ? `${connectionName} / ${path}` : path;
}
