import { lazy, Suspense } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { usePanelWidth, type PanelWidthOptions } from "../../hooks/usePanelWidth";
import { useInspector } from "../../stores/inspector";
import { useTabs, type Tab } from "../../stores/tabs";
import { TableDesignView } from "../design/TableDesignView";
import { TableDataView } from "../data/TableDataView";
import { InspectorPanel } from "../inspector/InspectorPanel";
import { MonitorView } from "../monitor/MonitorView";
import { ObjectListView } from "../objects/ObjectListView";
import { EmptyState } from "../ui/EmptyState";
import { ObjectTree } from "../sidebar/ObjectTree";
import { TabBar } from "../tabs/TabBar";
import { WelcomeView } from "../tabs/WelcomeView";
import { StatusBar } from "./StatusBar";
import { Toolbar } from "./Toolbar";

// SQL 编辑器(CodeMirror)只在打开查询标签页时才加载,不拖慢启动。
const QueryView = lazy(() => import("../query/QueryView").then((m) => ({ default: m.QueryView })));

const SIDEBAR: PanelWidthOptions = {
  storageKey: "basalt.sidebarWidth",
  defaultWidth: 260,
  min: 200,
  max: 480,
  edge: "right",
  label: "调整侧栏宽度",
};
const INSPECTOR: PanelWidthOptions = {
  storageKey: "basalt.inspectorWidth",
  defaultWidth: 280,
  min: 240,
  max: 600,
  edge: "left",
  label: "调整信息面板宽度",
};

export function AppShell() {
  const sidebar = usePanelWidth(SIDEBAR);
  const inspector = usePanelWidth(INSPECTOR);
  const connectionCount = useConnections().data?.length ?? 0;
  const inspectorOpen = useInspector((s) => s.open);

  return (
    <div
      className={inspectorOpen ? "app-shell has-inspector" : "app-shell"}
      style={
        {
          "--sidebar-width": `${sidebar.width}px`,
          "--inspector-width": `${inspectorOpen ? inspector.width : 0}px`,
        } as React.CSSProperties
      }
    >
      <Toolbar />
      <aside className="sidebar" aria-label="对象树">
        <div className="sidebar-header">
          <h2 className="sidebar-title">连接</h2>
          <span className="sidebar-meta">{connectionCount}</span>
        </div>
        <div className="sidebar-body">
          <ObjectTree />
        </div>
      </aside>
      <div className="panel-resizer sidebar-resizer" {...sidebar.separator} />
      <main className="workspace">
        <TabBar />
        <div className="workspace-body">
          <TabPanels />
        </div>
      </main>
      {inspectorOpen && <div className="panel-resizer inspector-resizer" {...inspector.separator} />}
      {inspectorOpen && <InspectorPanel />}
      <StatusBar />
    </div>
  );
}

/**
 * 所有标签页都保持挂载,只显示活动的那个:切走再切回来,未提交的修改、
 * 滚动位置和筛选条件都还在。
 */
function TabPanels() {
  const tabs = useTabs((s) => s.tabs);
  const activeId = useTabs((s) => s.activeId);
  if (tabs.length === 0) return <WelcomeView />;
  return (
    <>
      {tabs.map((tab) => (
        <div key={tab.id} className="tab-panel" hidden={tab.id !== activeId}>
          <TabContent tab={tab} active={tab.id === activeId} />
        </div>
      ))}
    </>
  );
}

function TabContent({ tab, active }: { tab: Tab; active: boolean }) {
  switch (tab.kind) {
    case "tableDesign":
      return <TableDesignView tab={tab} active={active} />;
    case "objects":
      return <ObjectListView tab={tab} />;
    case "tableData":
      return <TableDataView tab={tab} active={active} />;
    case "monitor":
      return <MonitorView tab={tab} active={active} />;
    case "query":
      return (
        <Suspense fallback={<EmptyState title="正在加载 SQL 编辑器…" />}>
          <QueryView tab={tab} active={active} />
        </Suspense>
      );
  }
}
