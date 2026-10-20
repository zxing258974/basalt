// 编辑器和结果区之间的分隔条:拖动改编辑器高度,双击恢复默认,高度作为个人偏好记在本机。
import { useCallback, useRef, useState, type PointerEvent } from "react";

const KEY = "basalt.queryEditorHeight";
export const EDITOR_DEFAULT = 260;
const EDITOR_MIN = 96;
/** 至少给结果区留这么高。 */
const RESULTS_MIN = 120;

function read(): number {
  try {
    const saved = Number(localStorage.getItem(KEY));
    return saved >= EDITOR_MIN ? saved : EDITOR_DEFAULT;
  } catch {
    return EDITOR_DEFAULT;
  }
}

function save(height: number): void {
  try {
    localStorage.setItem(KEY, String(Math.round(height)));
  } catch (error) {
    console.warn("无法保存编辑器高度", error);
  }
}

export function useEditorHeight() {
  const [height, setHeight] = useState(read);
  const drag = useRef<{ startY: number; startHeight: number; max: number; latest: number } | null>(null);

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      event.currentTarget.setPointerCapture(event.pointerId);
      const container = event.currentTarget.parentElement?.clientHeight ?? 0;
      const max = Math.max(EDITOR_MIN, container - RESULTS_MIN);
      drag.current = { startY: event.clientY, startHeight: height, max, latest: height };
    },
    [height],
  );
  const onPointerMove = useCallback((event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current) return;
    const next = Math.min(current.max, Math.max(EDITOR_MIN, current.startHeight + event.clientY - current.startY));
    current.latest = next;
    setHeight(next);
  }, []);
  const onPointerUp = useCallback(() => {
    if (drag.current) save(drag.current.latest);
    drag.current = null;
  }, []);
  const reset = useCallback(() => {
    setHeight(EDITOR_DEFAULT);
    save(EDITOR_DEFAULT);
  }, []);

  return { height, handlers: { onPointerDown, onPointerMove, onPointerUp, onDoubleClick: reset } };
}
