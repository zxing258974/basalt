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
