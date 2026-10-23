import { FilePlus2, Import, PanelRight, Pencil, Plug, PlugZap, Plus, RefreshCw, SquareTerminal } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { useNewQueryFromSelection } from "../../hooks/useNewQueryFromSelection";
import { useInspector } from "../../stores/inspector";
import { useUi } from "../../stores/ui";
import { ImportDialog } from "../import/ImportDialog";
import { connectionIdFromKey } from "../sidebar/treeKeys";
import { shortcut } from "../../lib/platform";

function ToolbarButton({
  icon,
  label,
  shortcut,
  disabled,
  pressed,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  /** 开关类按钮(如信息面板)的开合状态。 */
  pressed?: boolean;
  onClick(): void;
}) {
  return (
    <button
      className={pressed ? "toolbar-button is-pressed" : "toolbar-button"}
      disabled={disabled}
      aria-pressed={pressed}
      onClick={onClick}
      title={shortcut ? `${label}(${shortcut})` : label}
    >
      <span className="toolbar-button-icon">{icon}</span>
      <span className="toolbar-button-label">{label}</span>
    </button>
  );
}

export function Toolbar() {
  const selectedId = useUi((s) => connectionIdFromKey(s.selectedKey));
  const connection = useConnections().data?.find((c) => c.id === selectedId);
  const actions = useConnectionActions();
  const { openConnectionDialog } = useUi.getState();
  const [importing, setImporting] = useState(false);
  const newQuery = useNewQueryFromSelection();
  const inspectorOpen = useInspector((s) => s.open);
  const toggleInspector = useInspector((s) => s.toggle);

  return (
    <header className="toolbar" data-tauri-drag-region>
      <div className="toolbar-group" data-tauri-drag-region>
        <ToolbarButton
          icon={<Plus />}
          label="新建连接"
          shortcut={shortcut("N")}
          onClick={() => openConnectionDialog({ mode: "create", driver: null })}
        />
        {connection?.isOpen ? (
          <ToolbarButton
            icon={<Plug />}
            label="关闭连接"
            onClick={() => actions.withNotice("关闭连接", () => actions.close(connection.id))}
          />
        ) : (
          <ToolbarButton
            icon={<PlugZap />}
            label="打开连接"
            disabled={!connection}
            onClick={() => connection && void actions.openWithFeedback(connection)}
          />
        )}
        <ToolbarButton
          icon={<Pencil />}
          label="编辑"
          disabled={!connection}
          onClick={() => connection && openConnectionDialog({ mode: "edit", connection })}
        />
        <ToolbarButton
          icon={connection?.driver === "redis" ? <SquareTerminal /> : <FilePlus2 />}
          label={connection?.driver === "redis" ? "命令行" : "新建查询"}
          shortcut={shortcut("T")}
          disabled={!connection}
          onClick={() => void newQuery()}
        />
        <ToolbarButton icon={<Import />} label="导入" onClick={() => setImporting(true)} />
        <span className="toolbar-divider" aria-hidden />
        <ToolbarButton
          icon={<RefreshCw />}
          label="刷新"
          onClick={() =>
            actions.withNotice("刷新", () => actions.refresh(connection?.isOpen ? connection.id : undefined))
          }
        />
      </div>
      <div className="toolbar-group" data-tauri-drag-region>
        <ToolbarButton icon={<PanelRight />} label="信息" pressed={inspectorOpen} onClick={toggleInspector} />
        <span className="toolbar-divider" aria-hidden />
        <div className="toolbar-title" data-tauri-drag-region>
          Basalt
        </div>
      </div>
      {/* 拖动区域只认直接点在标题栏上的点击,对话框里的点击不会拖动窗口。 */}
      {importing && <ImportDialog onClose={() => setImporting(false)} />}
    </header>
  );
}
