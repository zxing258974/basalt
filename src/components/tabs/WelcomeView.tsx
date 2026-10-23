import { Plus } from "lucide-react";
import { useConnections } from "../../hooks/useCatalog";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { keyLabel, shortcut } from "../../lib/platform";

/** 快捷键按平台写:macOS 是 ⌘,Windows 是 Ctrl。 */
const shortcuts = (): [string, string][] => [
  [shortcut("N"), "新建连接"],
  [shortcut("T"), "在选中的库上新建查询（Redis 打开命令行）"],
  [shortcut("Enter"), `执行光标所在的语句（${shortcut("Enter", { shift: true })} 全部执行）`],
  [shortcut("W"), "关闭标签页"],
  ["↑ ↓ ← →", "在对象树里移动、展开、收起"],
  [keyLabel("Enter"), "打开选中的库或 schema"],
];

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
            ? `还没有连接。先新建一个，或者点工具栏的「导入」把 Navicat 里的连接搬过来；配置和密码都只保存在本机。`
            : "MySQL、MariaDB、PostgreSQL、SQLite、Redis。在左侧展开连接，双击库或 schema 打开它的对象列表，Redis 的库打开它的键。"}
        </p>
        <Button
          variant="primary"
          icon={<Plus />}
          onClick={() => openConnectionDialog({ mode: "create", driver: null })}
        >
          新建连接
        </Button>
        <dl className="welcome-shortcuts">
          {shortcuts().map(([keys, label]) => (
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
