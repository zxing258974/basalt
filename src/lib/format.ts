const UNKNOWN = "—";
const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;
const STEP = 1024;

const countFormatter = new Intl.NumberFormat("en-US");
const sizeFormatter = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

/** 1024 进制;KB 以上保留一位小数,整数时不带 .0。 */
export function formatBytes(bytes: number | null): string {
  if (bytes === null) return UNKNOWN;
  let value = bytes;
  let unit = 0;
  while (value >= STEP && unit < BYTE_UNITS.length - 1) {
    value /= STEP;
    unit += 1;
  }
  return `${sizeFormatter.format(value)} ${BYTE_UNITS[unit]}`;
}

export function formatCount(count: number | null): string {
  return count === null ? UNKNOWN : countFormatter.format(count);
}

const pad = (n: number) => String(n).padStart(2, "0");

/** 历史里的执行时间:今天只写时分秒,今年写月日,更早的带年份。按本机时区显示。 */
export function formatWhen(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  const time = `${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`;
  const sameYear = at.getFullYear() === now.getFullYear();
  if (sameYear && at.getMonth() === now.getMonth() && at.getDate() === now.getDate()) return time;
  const day = `${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
  return sameYear ? `${day} ${time.slice(0, 5)}` : `${at.getFullYear()}-${day}`;
}
