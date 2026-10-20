// 退出应用(⌘Q、点主窗口的关闭按钮)前,和关标签页一样确认未提交的表格修改、没保存的查询。
import { useEffect } from "react";
import { getBackend } from "../lib/api";
import { errorMessage } from "../lib/types";
import { guardQuit, hasUnsavedWork } from "../stores/discard";
import { useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";

function quit(): void {
  getBackend()
    .then((backend) => backend.quitApp())
    .catch((error: unknown) => useUi.getState().notify(`退出失败：${errorMessage(error)}`));
}

/**
 * 有没有未保存的东西,一变就报给后端。后端据此决定:没有就直接退出,不等前端(前端卡住了也退得掉);
 * 有就拦下来发事件过来,这里弹出和关标签页同一个确认框,确认放弃才真正退出。
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
    report(hasUnsavedWork(useTabs.getState()));
    const unsubscribe = useTabs.subscribe((state) => report(hasUnsavedWork(state)));
    const unlisten = getBackend().then((backend) => backend.onQuitRequested(() => guardQuit(quit)));
    return () => {
      unsubscribe();
      void unlisten.then((stop) => stop());
    };
  }, []);
}
