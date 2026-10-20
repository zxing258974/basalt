import { Plus } from "lucide-react";
import { useConnections } from "../../hooks/useCatalog";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";

const SHORTCUTS = [
  ["⌘N", "新建连接"],
  ["⌘T", "在选中的库上新建查询"],
  ["⌘↩", "执行光标所在的语句（⇧⌘↩ 全部执行）"],
  ["⌘W", "关闭标签页"],
  ["↑ ↓ ← →", "在对象树里移动、展开、收起"],
  ["↩", "打开选中的库或 schema"],
] as const;

/** 没有打开任何标签页时的首页。连接清单只在左侧侧栏里,这里不重复。 */
export function WelcomeView() {
  const query = useConnections();
  const { openConnectionDialog } = useUi.getState();
  const isEmpty = query.isSuccess && query.data.length === 0;

  return (
    <div className="welcome">
      <section className="welcome-hero">
        <p className="welcome-kicker">Basalt</p>
        <h1 className="welcome-title">
          连上库，
          <br />
          看清每一张表。
        </h1>
        <p className="welcome-lede">
          {isEmpty
            ? "还没有连接。先新建一个，或者点工具栏的「导入」把 Navicat 里的连接搬过来；配置只保存在本机，密码进钥匙串。"
            : "MySQL、MariaDB、PostgreSQL、SQLite。在左侧展开连接，双击库或 schema 打开它的对象列表。"}
        </p>
        <Button
          variant="primary"
          icon={<Plus />}
          onClick={() => openConnectionDialog({ mode: "create", driver: null })}
        >
          新建连接
        </Button>
        <dl className="welcome-shortcuts">
          {SHORTCUTS.map(([keys, label]) => (
            <div key={keys} className="welcome-shortcut">
              <dt>
                <kbd>{keys}</kbd>
              </dt>
              <dd>{label}</dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}
