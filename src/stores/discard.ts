// 关闭标签页、连接或退出应用之前,检查有没有未提交的表格修改、没保存的查询和表结构。
import { useTabs, type Tab } from "./tabs";
import { useUi, type DiscardPrompt } from "./ui";

type TabsSnapshot = Pick<ReturnType<typeof useTabs.getState>, "tabs" | "pending" | "unsaved">;
type LostWork = Pick<DiscardPrompt, "count" | "queries" | "designs">;

/** 关掉 `affects` 选出的标签页会丢掉的:未提交的表格修改行数、没保存的查询个数、表结构没保存的设计器个数。 */
function lostWork(state: TabsSnapshot, affects: (tab: Tab) => boolean): LostWork {
  const closing = state.tabs.filter(affects);
  return {
    count: closing.reduce((sum, tab) => sum + (state.pending[tab.id] ?? 0), 0),
    queries: closing.filter((tab) => tab.kind === "query" && state.unsaved[tab.id]).length,
    designs: closing.filter((tab) => tab.kind === "tableDesign" && state.unsaved[tab.id]).length,
  };
}

function nothingLost({ count, queries, designs }: LostWork): boolean {
  return count === 0 && queries === 0 && designs === 0;
}

/** 开着的标签页里有没有未提交的表格修改、没保存的查询或表结构。 */
export function hasUnsavedWork(state: TabsSnapshot): boolean {
  return !nothingLost(lostWork(state, () => true));
}

/** 退出会丢掉的:查询不算(下次启动原样恢复),只算未提交的表格修改和没保存的表结构。 */
export function blocksQuit(state: TabsSnapshot): boolean {
  return !nothingLost({ ...lostWork(state, () => true), queries: 0 });
}

function ask(
  affects: (tab: Tab) => boolean,
  request: Pick<DiscardPrompt, "onConfirm" | "onCancel" | "quit" | "action">,
  includeQueries = true,
): void {
  const all = lostWork(useTabs.getState(), affects);
  const lost = includeQueries ? all : { ...all, queries: 0 };
  if (nothingLost(lost)) {
    request.onConfirm();
    return;
  }
  // 退出请求随时会从原生菜单过来,可能正赶上别的确认框开着:先当它取消了,好让等它的一方收尾。
  const { discardPrompt, askDiscard } = useUi.getState();
  discardPrompt?.onCancel?.();
  askDiscard({ ...lost, ...request });
}

/**
 * `affects` 选出要被关掉的标签页:它们都没有未提交的修改就直接执行 `action`,
 * 否则弹出确认框,用户确认放弃后才执行。`verb` 是让修改没了的那个操作(默认「关闭」),确认框照它说。
 */
export function guardDiscard(
  affects: (tab: Tab) => boolean,
  action: () => void,
  onCancel?: () => void,
  verb?: string,
): void {
  ask(affects, { onConfirm: action, onCancel, action: verb });
}

/**
 * 退出应用之前:所有标签页一起算,确认框说「退出」而不是「关闭」。
 * 查询下次启动会恢复,默认不算;标签页没能记下来(`includeQueries`)时才把没保存的查询也算上。
 */
export function guardQuit(quit: () => void, { includeQueries = false }: { includeQueries?: boolean } = {}): void {
  ask(() => true, { onConfirm: quit, quit: true }, includeQueries);
}

/** 关掉一个标签页;有未提交的修改先确认。 */
export function closeTabGuarded(id: string): void {
  guardDiscard(
    (tab) => tab.id === id,
    () => useTabs.getState().close(id),
  );
}
