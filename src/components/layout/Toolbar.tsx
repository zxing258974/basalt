import { Pencil, Plug, PlugZap, Plus, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { useUi } from "../../stores/ui";
import { connectionIdFromKey } from "../sidebar/treeKeys";

function ToolbarButton({ icon, label, shortcut, disabled, onClick }: {
  icon: ReactNode;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  onClick(): void;
}) {
  return (
    <button
      className="toolbar-button"
      disabled={disabled}
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

  return (
    <header className="toolbar" data-tauri-drag-region>
      <div className="toolbar-group" data-tauri-drag-region>
        <ToolbarButton
          icon={<Plus />}
          label="新建连接"
          shortcut="⌘N"
          onClick={() => openConnectionDialog({ mode: "create", driver: null })}
        />
        {connection?.isOpen ? (
          <ToolbarButton icon={<Plug />} label="关闭连接" onClick={() => actions.withNotice("关闭连接", () => actions.close(connection.id))} />
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
        <span className="toolbar-divider" aria-hidden />
        <ToolbarButton
          icon={<RefreshCw />}
          label="刷新"
          onClick={() =>
            actions.withNotice("刷新", () => actions.refresh(connection?.isOpen ? connection.id : undefined))
          }
        />
      </div>
      <div className="toolbar-title" data-tauri-drag-region>
        Basalt
      </div>
    </header>
  );
}
