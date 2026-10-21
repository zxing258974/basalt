// 信息面板的纯逻辑:属性值的写法。
import type { PropertyValue } from "../../lib/types";
import { formatBytes, formatCount } from "../transfer/exportModel";

/** 字节写成「16 KB（16,384）」:一眼看出量级,也看得到精确值。 */
export function formatProperty(value: PropertyValue | null): string {
  if (value === null) return "—";
  switch (value.kind) {
    case "text":
    case "dateTime":
      return value.value;
    case "count":
      return formatCount(value.value);
    case "bytes":
      return value.value < 1024 ? `${value.value} B` : `${formatBytes(value.value)}（${formatCount(value.value)}）`;
  }
}
