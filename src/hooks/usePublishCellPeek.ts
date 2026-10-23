// 表格把选中的那一格报给右侧信息面板的「值」页。只在所在的标签页正显示着时报:
// 切回来时重报一次,卸载(换了结果集、重新执行)时清掉自己报的。
import { useContext, useEffect, useId } from "react";
import { TabScope } from "../components/layout/tabScope";
import { useCellPeek, type CellPeek } from "../stores/cellPeek";

/** `peek` 要 memo 过:每次渲染都是新对象的话,每次渲染都会报一次。 */
export function usePublishCellPeek(peek: CellPeek | null): void {
  const scope = useContext(TabScope);
  const owner = useId();
  const tabId = scope?.tabId;
  const active = scope?.active ?? false;
  useEffect(() => {
    if (tabId !== undefined && active) useCellPeek.getState().publish(tabId, owner, peek);
  }, [active, owner, peek, tabId]);
  useEffect(() => () => useCellPeek.getState().release(owner), [owner]);
}
