import type { RedisKind } from "../../lib/redisTypes";
import { KIND_NAMES } from "./redisKeyModel";

const SHORT: Record<string, string> = {
  string: "STR",
  hash: "HASH",
  list: "LIST",
  set: "SET",
  zset: "ZSET",
  stream: "XSTR",
};

/** 键的类型:缩写加颜色,鼠标停上去看中文说明。模块的类型(RedisJSON 等)灰色显示原名。 */
export function KindBadge({ kind, size = "sm" }: { kind: RedisKind; size?: "sm" | "md" }) {
  const known = kind in SHORT;
  return (
    <span
      className={`kind-badge is-${size} ${known ? `kind-${kind}` : "kind-other"}`}
      title={known ? `${kind}（${KIND_NAMES[kind]}）` : kind}
    >
      {size === "md" ? kind : (SHORT[kind] ?? kind.slice(0, 4).toUpperCase())}
    </span>
  );
}
