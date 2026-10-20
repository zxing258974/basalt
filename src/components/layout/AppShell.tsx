import { useCallback, useRef, useState, type PointerEvent } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useTabs, type Tab } from "../../stores/tabs";
import { TableDesignView } from "../design/TableDesignView";
import { TableDataView } from "../data/TableDataView";
import { ObjectListView } from "../objects/ObjectListView";
import { ObjectTree } from "../sidebar/ObjectTree";
import { TabBar } from "../tabs/TabBar";
import { WelcomeView } from "../tabs/WelcomeView";
import { StatusBar } from "./StatusBar";
import { Toolbar } from "./Toolbar";

const SIDEBAR_WIDTH_KEY = "basalt.sidebarWidth";
const SIDEBAR_DEFAULT = 260;
const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 480;

const clampWidth = (width: number) => Math.min(Math.max(width, SIDEBAR_MIN), SIDEBAR_MAX);

/** 侧栏宽度只是个人偏好:读写失败(隐私模式等)就用默认值。 */
function readSidebarWidth(): number {
  try {
    const saved = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
    return saved ? clampWidth(saved) : SIDEBAR_DEFAULT;
  } catch {
    return SIDEBAR_DEFAULT;
  }
}

function saveSidebarWidth(width: number): void {
  try {
    localStorage.setItem(SIDEBAR_WIDTH_KEY, String(width));
  } catch (error) {
    console.warn("无法保存侧栏宽度", error);
  }
}

export function AppShell() {
  const [width, setWidth] = useState(readSidebarWidth);
  const drag = useRef<{ startX: number; startWidth: number; latest: number } | null>(null);
  const connectionCount = useConnections().data?.length ?? 0;

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      event.currentTarget.setPointerCapture(event.pointerId);
      drag.current = { startX: event.clientX, startWidth: width, latest: width };
    },
    [width],
  );
  const onPointerMove = useCallback((event: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const next = clampWidth(drag.current.startWidth + event.clientX - drag.current.startX);
    drag.current.latest = next;
    setWidth(next);
  }, []);
  const onPointerUp = useCallback(() => {
    if (!drag.current) return;
    saveSidebarWidth(drag.current.latest);
    drag.current = null;
  }, []);

  return (
    <div className="app-shell" style={{ "--sidebar-width": `${width}px` } as React.CSSProperties}>
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
      <div
        className="sidebar-resizer"
        role="separator"
        aria-orientation="vertical"
        aria-label="调整侧栏宽度"
        aria-valuenow={width}
        aria-valuemin={SIDEBAR_MIN}
        aria-valuemax={SIDEBAR_MAX}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onDoubleClick={() => {
          setWidth(SIDEBAR_DEFAULT);
          saveSidebarWidth(SIDEBAR_DEFAULT);
        }}
      />
      <main className="workspace">
        <TabBar />
        <div className="workspace-body">
          <TabPanels />
        </div>
      </main>
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
  }
}
