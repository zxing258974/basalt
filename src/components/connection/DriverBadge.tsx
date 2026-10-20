import type { DriverKind } from "../../lib/types";
import { driverMeta } from "./driverMeta";

interface DriverBadgeProps {
  driver: DriverKind;
  size?: "sm" | "lg";
  /** 已打开的连接在徽标右下角亮一个点。 */
  live?: boolean;
}

export function DriverBadge({ driver, size = "sm", live = false }: DriverBadgeProps) {
  const meta = driverMeta(driver);
  return (
    <span
      className={`driver-badge driver-badge-${size}`}
      style={{ "--badge-color": meta.color } as React.CSSProperties}
      aria-hidden
    >
      {meta.monogram}
      {live && <span className="driver-badge-live" />}
    </span>
  );
}
