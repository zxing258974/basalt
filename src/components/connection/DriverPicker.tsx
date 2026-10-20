import type { DriverKind } from "../../lib/types";
import { DriverBadge } from "./DriverBadge";
import { DRIVERS } from "./driverMeta";

interface DriverPickerProps {
  onPick(driver: DriverKind): void;
}

export function DriverPicker({ onPick }: DriverPickerProps) {
  return (
    <div className="driver-picker" role="list">
      {DRIVERS.map((driver) => (
        <button
          key={driver.kind}
          role="listitem"
          className="driver-card"
          style={{ "--badge-color": driver.color } as React.CSSProperties}
          onClick={() => onPick(driver.kind)}
        >
          <DriverBadge driver={driver.kind} size="lg" />
          <span className="driver-card-label">{driver.label}</span>
          <span className="driver-card-meta mono">
            {driver.defaultPort ? `:${driver.defaultPort}` : "本地文件"}
          </span>
        </button>
      ))}
    </div>
  );
}
