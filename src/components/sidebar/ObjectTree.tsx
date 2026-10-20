import { Copy, Folder, Pencil, Plug, PlugZap, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useMemo, useState, type KeyboardEvent, type MouseEvent } from "react";
import { useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { errorMessage, type ConnectionView } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { DriverBadge } from "../connection/DriverBadge";
import { endpointSummary } from "../connection/driverMeta";
import { ContextMenu, type MenuEntry } from "../ui/ContextMenu";
import { DatabaseList } from "./CatalogNodes";
import { connectionKey, folderKey } from "./treeKeys";
import { TreeMenuContext, useTreeMenu } from "./treeMenu";
import { TreeRow, TreeStatusRow } from "./TreeRow";

interface MenuState {
  x: number;
  y: number;
  items: MenuEntry[];
}

/** 按「分组」字段归类,未分组的排在最前。保持原有顺序。 */
function groupConnections(connections: ConnectionView[]) {
  const ungrouped = connections.filter((c) => !c.group);
  const groups = new Map<string, ConnectionView[]>();
  for (const c of connections) {
    if (c.group) groups.set(c.group, [...(groups.get(c.group) ?? []), c]);
  }
  return { ungrouped, groups: [...groups.entries()] };
}

/** 上下方向键在所有可见节点间按文档顺序移动。 */
function moveFocus(event: KeyboardEvent<HTMLDivElement>) {
  const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
  if (!keys.includes(event.key)) return;
  event.preventDefault();
  const items = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="treeitem"]')];
  const current = items.indexOf(document.activeElement as HTMLElement);
  const target =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? items.length - 1
        : Math.min(Math.max(current + (event.key === "ArrowDown" ? 1 : -1), 0), items.length - 1);
  items[target]?.focus();
}

export function ObjectTree() {
  const query = useConnections();
  const [menu, setMenu] = useState<MenuState | null>(null);
  const openMenu = useCallback(
    (event: MouseEvent, items: MenuEntry[]) => setMenu({ x: event.clientX, y: event.clientY, items }),
    [],
  );
  const closeMenu = useCallback(() => setMenu(null), []);
  const grouped = useMemo(() => groupConnections(query.data ?? []), [query.data]);

  return (
    <TreeMenuContext.Provider value={openMenu}>
      <div role="tree" aria-label="连接" className="tree" onKeyDown={moveFocus}>
        {query.isPending && <TreeStatusRow depth={0} status="loading" message="读取连接…" />}
        {query.error && (
          <TreeStatusRow
            depth={0}
            status="error"
            message={errorMessage(query.error)}
            onRetry={() => void query.refetch()}
          />
        )}
        {grouped.ungrouped.map((connection) => (
          <ConnectionNode key={connection.id} connection={connection} depth={0} />
        ))}
        {grouped.groups.map(([group, members]) => (
          <TreeRow
            key={group}
            nodeKey={folderKey(group)}
            depth={0}
            label={group}
            icon={<Folder />}
            count={members.length}
            expandable
          >
            {members.map((connection) => (
              <ConnectionNode key={connection.id} connection={connection} depth={1} />
            ))}
          </TreeRow>
        ))}
      </div>
      {menu && <ContextMenu {...menu} onClose={closeMenu} />}
    </TreeMenuContext.Provider>
  );
}

function ConnectionNode({ connection, depth }: { connection: ConnectionView; depth: number }) {
  const openMenu = useTreeMenu();
  const actions = useConnectionActions();
  const { openConnectionDialog, confirmDelete } = useUi.getState();
  const error = useUi((s) => s.connectionErrors[connection.id]);
  const openSafely = () => actions.openWithFeedback(connection);

  const items: MenuEntry[] = [
    connection.isOpen
      ? {
          label: "关闭连接",
          icon: <Plug />,
          onSelect: () => actions.withNotice("关闭连接", () => actions.close(connection.id)),
        }
      : { label: "打开连接", icon: <PlugZap />, onSelect: () => void openSafely() },
    "separator",
    { label: "编辑连接…", icon: <Pencil />, onSelect: () => openConnectionDialog({ mode: "edit", connection }) },
    {
      label: "复制连接",
      icon: <Copy />,
      onSelect: () => actions.withNotice("复制连接", () => actions.duplicate(connection.id)),
    },
    {
      label: "刷新",
      icon: <RefreshCw />,
      disabled: !connection.isOpen,
      onSelect: () => actions.withNotice("刷新", () => actions.refresh(connection.id)),
    },
    "separator",
    { label: "删除连接…", icon: <Trash2 />, danger: true, onSelect: () => confirmDelete(connection) },
  ];

  return (
    <TreeRow
      nodeKey={connectionKey(connection.id)}
      depth={depth}
      label={
        <span className={connection.isOpen ? "connection-name is-open" : "connection-name"}>{connection.name}</span>
      }
      accessibleName={connection.name}
      title={endpointSummary(connection)}
      icon={<DriverBadge driver={connection.driver} live={connection.isOpen} />}
      expandable
      onBeforeExpand={openSafely}
      onContextMenu={(event) => openMenu(event, items)}
    >
      {error ? (
        <TreeStatusRow depth={depth + 1} status="error" message={error} onRetry={() => void openSafely()} />
      ) : (
        connection.isOpen && <DatabaseList connection={connection} depth={depth + 1} />
      )}
    </TreeRow>
  );
}
