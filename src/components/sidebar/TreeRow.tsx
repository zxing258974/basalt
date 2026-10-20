import { ChevronRight } from "lucide-react";
import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
import { useUi } from "../../stores/ui";

interface TreeRowProps {
  nodeKey: string;
  depth: number;
  label: ReactNode;
  /** 读屏软件念出的名字;label 不是纯文本时必须提供。 */
  accessibleName?: string;
  icon?: ReactNode;
  /** 鼠标悬停提示。 */
  title?: string;
  /** 行尾的灰色计数。 */
  count?: number;
  expandable?: boolean;
  /** 展开前的钩子,返回 false 取消展开(如连接需要先输密码)。 */
  onBeforeExpand?: () => Promise<boolean> | boolean;
  /** 双击或回车;未提供时等同于切换展开。 */
  onActivate?: () => void;
  onContextMenu?: (event: MouseEvent) => void;
  children?: ReactNode;
}

const focusTreeItem = (element: Element | null | undefined) => (element as HTMLElement | null)?.focus();

export function TreeRow({
  nodeKey,
  depth,
  label,
  accessibleName,
  icon,
  title,
  count,
  expandable = false,
  onBeforeExpand,
  onActivate,
  onContextMenu,
  children,
}: TreeRowProps) {
  const expanded = useUi((s) => Boolean(s.expanded[nodeKey]));
  const selected = useUi((s) => s.selectedKey === nodeKey);
  const { setExpanded, select } = useUi.getState();

  const setOpen = async (next: boolean) => {
    if (next && onBeforeExpand && !(await onBeforeExpand())) return;
    setExpanded(nodeKey, next);
  };
  const activate = () => {
    if (onActivate) onActivate();
    else if (expandable) void setOpen(!expanded);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    const item = event.currentTarget;
    switch (event.key) {
      case "ArrowRight":
        event.preventDefault();
        if (expandable && !expanded) void setOpen(true);
        else focusTreeItem(item.querySelector('[role="group"] [role="treeitem"]'));
        break;
      case "ArrowLeft":
        event.preventDefault();
        if (expandable && expanded) setExpanded(nodeKey, false);
        else focusTreeItem(item.parentElement?.closest('[role="treeitem"]'));
        break;
      case "Enter":
        event.preventDefault();
        activate();
        break;
    }
  };

  return (
    <div
      role="treeitem"
      aria-label={accessibleName ?? (typeof label === "string" ? label : undefined)}
      aria-level={depth + 1}
      aria-expanded={expandable ? expanded : undefined}
      aria-selected={selected}
      tabIndex={selected ? 0 : -1}
      data-key={nodeKey}
      className="tree-item"
      onFocus={(event) => event.target === event.currentTarget && select(nodeKey)}
      onKeyDown={onKeyDown}
    >
      <div
        className={selected ? "tree-row is-selected" : "tree-row"}
        style={{ "--depth": depth } as React.CSSProperties}
        title={title}
        onClick={(event) => {
          select(nodeKey);
          (event.currentTarget.parentElement as HTMLElement).focus();
        }}
        onDoubleClick={activate}
        onContextMenu={(event) => {
          event.preventDefault();
          select(nodeKey);
          onContextMenu?.(event);
        }}
      >
        <span
          className={expanded ? "tree-twisty is-open" : "tree-twisty"}
          aria-hidden
          onClick={(event) => {
            event.stopPropagation();
            if (expandable) void setOpen(!expanded);
          }}
        >
          {expandable && <ChevronRight />}
        </span>
        {icon && <span className="tree-icon">{icon}</span>}
        <span className="tree-label">{label}</span>
        {count !== undefined && <span className="tree-count mono">{count}</span>}
      </div>
      {expandable && expanded && children && (
        <div role="group" className="tree-group">
          {children}
        </div>
      )}
    </div>
  );
}

/** 子节点的加载中 / 出错占位行。 */
export function TreeStatusRow({
  depth,
  status,
  message,
  onRetry,
}: {
  depth: number;
  status: "loading" | "error" | "empty";
  message: string;
  onRetry?: () => void;
}) {
  return (
    <div className={`tree-row tree-status is-${status}`} style={{ "--depth": depth } as React.CSSProperties}>
      <span className="tree-twisty" />
      {status === "loading" && <span className="spinner" aria-hidden />}
      <span className="tree-label" title={message}>
        {message}
      </span>
      {onRetry && (
        <button className="tree-retry" onClick={onRetry}>
          重试
        </button>
      )}
    </div>
  );
}
