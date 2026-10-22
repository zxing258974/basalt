import { useState, type PointerEvent } from "react";
import { HISTORY_LIMIT, formatClock } from "./monitorModel";

const HEIGHT = 36;
/** 线条上下留的空,免得 2px 的线贴边被裁掉一半。 */
const INSET = 3;

interface SparklineProps {
  label: string;
  values: readonly (number | null)[];
  /** 每个点的时刻(Unix 毫秒),和 values 一一对应。 */
  times: readonly number[];
  format(value: number): string;
  /** 速率从 0 画起;命中率这类总在高位的按实际范围画,不然是一条贴顶的直线。 */
  zeroBased?: boolean;
  limit?: number;
}

function domain(values: readonly number[], zeroBased: boolean): [number, number] {
  const max = Math.max(...values);
  if (zeroBased) return [0, max > 0 ? max * 1.15 : 1];
  const min = Math.min(...values);
  const pad = max > min ? (max - min) * 0.15 : Math.max(Math.abs(max) * 0.001, 0.0001);
  return [min - pad, max + pad];
}

/**
 * 一个指标最近一段的走势。点从右往左排:最新的在最右边,还没攒满时左边空着。
 * 线用 SVG(横向拉伸、线宽不变),末端圆点和悬停的十字线用 HTML 叠在上面,不跟着变形。
 */
export function Sparkline({ label, values, times, format, zeroBased = true, limit = HISTORY_LIMIT }: SparklineProps) {
  const [hover, setHover] = useState<number | null>(null);
  const present = values.filter((v): v is number => v !== null);
  const offset = limit - values.length;
  const xPercent = (index: number) => ((offset + index) / (limit - 1)) * 100;

  if (present.length === 0) {
    return <div className="sparkline is-empty" aria-hidden />;
  }
  const [low, high] = domain(present, zeroBased);
  const y = (value: number) => INSET + (1 - (value - low) / (high - low)) * (HEIGHT - INSET * 2);

  // 读不到的点断开,不连成一条假的线。
  const segments: string[] = [];
  let current: string[] = [];
  values.forEach((value, index) => {
    if (value === null) {
      if (current.length) segments.push(current.join(" "));
      current = [];
      return;
    }
    current.push(`${current.length ? "L" : "M"}${xPercent(index).toFixed(3)},${y(value).toFixed(2)}`);
  });
  if (current.length) segments.push(current.join(" "));
  const line = segments.join(" ");

  const lastIndex = values.length - 1;
  const lastValue = values[lastIndex];
  const hovered = hover !== null ? values[hover] : null;

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width <= 0) return;
    const slot = Math.round(((event.clientX - rect.left) / rect.width) * (limit - 1));
    const index = Math.min(values.length - 1, Math.max(0, slot - offset));
    setHover(values[index] === null ? null : index);
  };

  return (
    <div
      className="sparkline"
      role="img"
      aria-label={`${label}最近 ${values.length} 次的走势`}
      onPointerMove={onPointerMove}
      onPointerLeave={() => setHover(null)}
    >
      <svg viewBox={`0 0 100 ${HEIGHT}`} preserveAspectRatio="none" aria-hidden>
        <path className="sparkline-line" d={line} vectorEffect="non-scaling-stroke" />
      </svg>
      {lastValue !== null && lastValue !== undefined && hover === null && (
        <span className="sparkline-dot" style={{ left: `${xPercent(lastIndex)}%`, top: y(lastValue) }} />
      )}
      {hover !== null && hovered !== null && hovered !== undefined && (
        <>
          <span className="sparkline-crosshair" style={{ left: `${xPercent(hover)}%` }} />
          <span className="sparkline-dot" style={{ left: `${xPercent(hover)}%`, top: y(hovered) }} />
          <span
            className={xPercent(hover) > 60 ? "sparkline-tooltip is-left" : "sparkline-tooltip"}
            style={{ left: `${xPercent(hover)}%` }}
            role="tooltip"
          >
            <strong>{format(hovered)}</strong>
            <span>{times[hover] !== undefined ? formatClock(times[hover]) : ""}</span>
          </span>
        </>
      )}
    </div>
  );
}
