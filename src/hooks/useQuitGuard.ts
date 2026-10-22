// 退出应用(⌘Q、点主窗口的关闭按钮)前:先把开着的标签页记下来(下次启动恢复,没保存的查询也在),
// 再和关标签页一样确认未提交的表格修改、没保存的表结构——这两样不会恢复。
import { useEffect } from "react";
import { getBackend } from "../lib/api";
import { errorMessage } from "../lib/types";
import { blocksQuit, guardQuit } from "../stores/discard";
import { useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";
import { flushWorkspace, useWorkspace } from "../stores/workspace";

function quit(): void {
  getBackend()
    .then((backend) => backend.quitApp())
    .catch((error: unknown) => useUi.getState().notify(`退出失败：${errorMessage(error)}`));
}

/** 退出前要不要先问前端:有会丢的修改,或者标签页还有改动没写进记录。 */
const needsFrontend = () => blocksQuit(useTabs.getState()) || useWorkspace.getState().pending;

/**
 * 退出前要不要先问前端,一变就报给后端。后端据此决定:不用就直接退出,不等前端(前端卡住了也退得掉);
 * 要的话拦下来发事件过来,这里先把标签页记下来,再弹出和关标签页同一个确认框,确认放弃才真正退出。
 */
export function useQuitGuard(): void {
  useEffect(() => {
    let reported: boolean | null = null;
    // 一条接一条发,免得先发的后到、盖掉后发的。
    let queue = Promise.resolve();
    const report = (unsaved: boolean) => {
      if (unsaved === reported) return;
      reported = unsaved;
      queue = queue
        .then(async () => (await getBackend()).setUnsavedWork(unsaved))
        .catch((error: unknown) => {
          reported = null; // 下次有变化时重报
          useUi.getState().notify(`退出时的未保存提醒可能不准：${errorMessage(error)}`);
        });
    };
    report(needsFrontend());
    const stopTabs = useTabs.subscribe(() => report(needsFrontend()));
    const stopWorkspace = useWorkspace.subscribe(() => report(needsFrontend()));
    // 没能记下来的话,没保存的查询下次就回不来了:这时也把它们算进确认框。
    const onQuit = () => void flushWorkspace().then((saved) => guardQuit(quit, { includeQueries: !saved }));
    const unlisten = getBackend().then((backend) => backend.onQuitRequested(onQuit));
    return () => {
      stopTabs();
      stopWorkspace();
      void unlisten.then((stop) => stop());
    };
  }, []);
}
