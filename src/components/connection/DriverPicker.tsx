import type { DriverKind } from "../../lib/types";
import { DriverBadge } from "./DriverBadge";
import { driverMeta } from "./driverMeta";

interface DriverPickerProps {
  onPick(driver: DriverKind): void;
}

/** SQL 数据库和 Redis 能做的事不一样(表和 SQL / 键和命令),分两组。 */
const GROUPS: readonly { title: string; kinds: readonly DriverKind[]; note?: string }[] = [
  { title: "SQL 数据库", kinds: ["mysql", "mariadb", "postgres", "sqlite"] },
  { title: "键值存储", kinds: ["redis"], note: "浏览和编辑键、命令行、服务器监控" },
];

export function DriverPicker({ onPick }: DriverPickerProps) {
  return (
    <div className="driver-picker">
      {GROUPS.map((group) => (
        <section key={group.title} className="driver-group" aria-label={group.title}>
          <h3 className="driver-group-title">{group.title}</h3>
          <div className={group.kinds.length === 1 ? "driver-grid is-single" : "driver-grid"} role="list">
            {group.kinds.map((kind) => {
              const driver = driverMeta(kind);
              return (
                <button
                  key={kind}
                  role="listitem"
                  className="driver-card"
                  style={{ "--badge-color": driver.color } as React.CSSProperties}
                  onClick={() => onPick(kind)}
                >
                  <DriverBadge driver={kind} size="lg" />
                  <span className="driver-card-label">{driver.label}</span>
                  <span className="driver-card-meta mono">
                    {driver.defaultPort ? `:${driver.defaultPort}` : "本地文件"}
                  </span>
                  {group.note && group.kinds.length === 1 && <span className="driver-card-note">{group.note}</span>}
                </button>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
