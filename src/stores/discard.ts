// 关闭标签页或连接之前,检查有没有未提交的表格修改。
import { useTabs, type Tab } from "./tabs";
import { useUi } from "./ui";

/**
 * `affects` 选出要被关掉的标签页:它们都没有未提交的修改就直接执行 `action`,
 * 否则弹出确认框,用户确认放弃后才执行。
 */
export function guardDiscard(affects: (tab: Tab) => boolean, action: () => void, onCancel?: () => void): void {
  const { tabs, pending } = useTabs.getState();
  const count = tabs.filter(affects).reduce((sum, tab) => sum + (pending[tab.id] ?? 0), 0);
  if (count === 0) {
    action();
    return;
  }
  useUi.getState().askDiscard({ count, onConfirm: action, onCancel });
}

/** 关掉一个标签页;有未提交的修改先确认。 */
export function closeTabGuarded(id: string): void {
  guardDiscard((tab) => tab.id === id, () => useTabs.getState().close(id));
}
