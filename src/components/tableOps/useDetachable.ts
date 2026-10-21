// 可能要跑很久的表操作(大表的清空、OPTIMIZE、VACUUM):执行中可以先关掉对话框,操作在后台做完。
import { useCallback, useRef } from "react";

export function useDetachable(onClose: () => void) {
  const detached = useRef(false);
  /** 关掉对话框,操作接着跑。 */
  const detach = useCallback(() => {
    detached.current = true;
    onClose();
  }, [onClose]);
  /** 对话框是不是已经先关掉了:是的话结果改用状态栏提示,也不能再调 onClose(那会关掉后来打开的对话框)。 */
  const isDetached = useCallback(() => detached.current, []);
  return { detach, isDetached };
}
