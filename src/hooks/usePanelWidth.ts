// 侧边面板的分隔条:拖动改宽度,双击恢复默认,方向键微调;宽度作为个人偏好记在本机。
// 左侧栏的分隔条在面板右边(往右拖变宽),右侧面板的在左边(往左拖变宽)。
import { useCallback, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

export interface PanelWidthOptions {
  storageKey: string;
  defaultWidth: number;
  min: number;
  max: number;
  /** 分隔条在面板的哪一边。 */
  edge: "left" | "right";
  label: string;
}

/** 拖宽面板时,旁边能伸缩的那块(工作区、编辑器)至少留这么宽。 */
export const REST_MIN = 320;
const KEY_STEP = 16;
const KEY_STEP_LARGE = 64;

const clamp = (width: number, min: number, max: number) => Math.min(Math.max(width, min), max);

function read({ storageKey, defaultWidth, min, max }: PanelWidthOptions): number {
  try {
    const saved = Number(localStorage.getItem(storageKey));
    return saved ? clamp(saved, min, max) : defaultWidth;
  } catch {
    return defaultWidth;
  }
}

function save(key: string, width: number): void {
  try {
    localStorage.setItem(key, String(Math.round(width)));
  } catch (error) {
    console.warn("无法保存面板宽度", error);
  }
}

/** 开始拖的时候量一次:面板实际多宽、旁边那块还能让出多少。没排版(量出 0)就按记下的宽度和上限来。 */
function measure(separator: HTMLElement, edge: PanelWidthOptions["edge"], width: number, max: number) {
  const panel = edge === "left" ? separator.nextElementSibling : separator.previousElementSibling;
  const rest = edge === "left" ? separator.previousElementSibling : separator.nextElementSibling;
  const shown = panel?.getBoundingClientRect().width || width;
  const restWidth = rest?.getBoundingClientRect().width ?? 0;
  return { start: shown, max: restWidth > 0 ? Math.min(max, shown + Math.max(0, restWidth - REST_MIN)) : max };
}

export function usePanelWidth(options: PanelWidthOptions) {
  const { storageKey, defaultWidth, min, max, edge, label } = options;
  const [width, setWidth] = useState(() => read(options));
  const drag = useRef<{ startX: number; start: number; max: number; latest: number } | null>(null);
  const sign = edge === "right" ? 1 : -1;

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      event.currentTarget.setPointerCapture(event.pointerId);
      const measured = measure(event.currentTarget, edge, width, max);
      drag.current = { startX: event.clientX, ...measured, latest: width };
    },
    [edge, width, max],
  );
  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const current = drag.current;
      if (!current) return;
      const next = clamp(current.start + sign * (event.clientX - current.startX), min, Math.max(min, current.max));
      current.latest = next;
      setWidth(next);
    },
    [sign, min],
  );
  const onPointerUp = useCallback(() => {
    if (drag.current) save(storageKey, drag.current.latest);
    drag.current = null;
  }, [storageKey]);
  const reset = useCallback(() => {
    setWidth(defaultWidth);
    save(storageKey, defaultWidth);
  }, [storageKey, defaultWidth]);
  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      const step = event.shiftKey ? KEY_STEP_LARGE : KEY_STEP;
      const moves: Record<string, number> = { ArrowLeft: -step, ArrowRight: step };
      if (!(event.key in moves) && event.key !== "Home" && event.key !== "End") return;
      event.preventDefault();
      const measured = measure(event.currentTarget, edge, width, max);
      const cap = Math.max(min, measured.max);
      const next =
        event.key === "Home"
          ? min
          : event.key === "End"
            ? cap
            : clamp(measured.start + sign * moves[event.key]!, min, cap);
      setWidth(next);
      save(storageKey, next);
    },
    [edge, width, sign, min, max, storageKey],
  );

  return {
    width,
    separator: {
      role: "separator",
      tabIndex: 0,
      "aria-orientation": "vertical" as const,
      "aria-label": label,
      "aria-valuenow": Math.round(width),
      "aria-valuemin": min,
      "aria-valuemax": max,
      title: "拖动调整宽度，双击恢复默认",
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onDoubleClick: reset,
      onKeyDown,
    },
  };
}
