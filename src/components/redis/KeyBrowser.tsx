import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronRight, Clock, FolderTree, List, Plus, RefreshCw, Search, X } from "lucide-react";
import { useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useRedisKeyspace } from "../../hooks/useCatalog";
import { REDIS_KINDS, type RedisKind } from "../../lib/redisTypes";
import { errorMessage } from "../../lib/types";
import { Button } from "../ui/Button";
import { KindBadge } from "./KindBadge";
import { browserRows, formatTtl, mergeKeys, scanPattern, type BrowserRow } from "./redisKeyModel";
import { useKeyScan } from "./useRedisData";

const ROW_HEIGHT = 28;
const INDENT = 14;

/** 一行是谁:展开折叠、加载下一批之后行号会变,键盘游标按它找回原来那一行。 */
const rowId = (row: BrowserRow) => (row.kind === "key" ? row.key.key.id : `folder:${row.path}`);

interface KeyBrowserProps {
  connectionId: string;
  db: number;
  selected: string | null;
  onSelect(key: string): void;
  onNewKey(prefix: string): void;
}

/** 左边的键列表:按名字和类型筛选,按冒号分组或平铺,一批一批往下扫。 */
export function KeyBrowser({ connectionId, db, selected, onSelect, onNewKey }: KeyBrowserProps) {
  const [draft, setDraft] = useState("");
  const [filter, setFilter] = useState("");
  const [kind, setKind] = useState<RedisKind | null>(null);
  const [grouped, setGrouped] = useState(true);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [cursor, setCursor] = useState<string | null>(null);
  const scan = useKeyScan(connectionId, db, scanPattern(filter), kind);
  const total = useRedisKeyspace(connectionId, true).data?.databases.find((d) => d.index === db)?.keys;

  const keys = useMemo(() => mergeKeys(scan.data?.pages.map((p) => p.keys) ?? []), [scan.data]);
  // 筛选时全部展开:匹配到的键常在很深的分组里。
  const rows = useMemo(
    () => browserRows(keys, grouped, filter || kind ? "all" : expanded),
    [expanded, filter, grouped, keys, kind],
  );
  // 游标那一行不在了(删了、筛掉了)就落到选中的键上,再不行落到第一行。
  const cursorAt = cursor === null ? -1 : rows.findIndex((r) => rowId(r) === cursor);
  const active =
    cursorAt >= 0
      ? cursorAt
      : Math.max(
          rows.findIndex((r) => r.kind === "key" && r.key.key.id === selected),
          0,
        );

  const listId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 16,
  });

  const toggle = (path: string, open?: boolean) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (open ?? !next.has(path)) next.add(path);
      else next.delete(path);
      return next;
    });

  const activate = (row: BrowserRow) => {
    setCursor(rowId(row));
    if (row.kind === "key") onSelect(row.key.key.id);
    else toggle(row.path);
  };

  const move = (index: number) => {
    const next = Math.min(Math.max(index, 0), rows.length - 1);
    const row = rows[next];
    if (!row) return;
    setCursor(rowId(row));
    virtualizer.scrollToIndex(next, { align: "auto" });
    if (row.kind === "key") onSelect(row.key.key.id);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    const row = rows[active];
    switch (event.key) {
      case "ArrowDown":
        move(active + 1);
        break;
      case "ArrowUp":
        move(active - 1);
        break;
      case "Home":
        move(0);
        break;
      case "End":
        move(rows.length - 1);
        break;
      case "ArrowRight":
        if (row?.kind === "folder" && !row.expanded) toggle(row.path, true);
        break;
      case "ArrowLeft":
        if (row?.kind === "folder" && row.expanded) toggle(row.path, false);
        break;
      case "Enter":
        if (row) activate(row);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const applyFilter = (text: string) => {
    setFilter(text.trim());
    setCursor(null);
  };
  const folderOfActive = rows[active]?.kind === "folder" ? `${(rows[active] as { path: string }).path}:` : "";

  return (
    <section className="key-browser" aria-label="键">
      <div className="key-browser-bar">
        <label className="key-filter">
          <Search aria-hidden />
          <input
            value={draft}
            placeholder="按名字找，可用 *"
            title="回车执行。* 匹配任意一串字，? 匹配一个字，[abc] 匹配其中一个"
            aria-label="按名字筛选键"
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") applyFilter(draft);
              if (event.key === "Escape" && draft) {
                setDraft("");
                applyFilter("");
              }
            }}
          />
          {(draft || filter) && (
            <button
              type="button"
              className="key-filter-clear"
              aria-label="清除筛选"
              onClick={() => {
                setDraft("");
                applyFilter("");
              }}
            >
              <X aria-hidden />
            </button>
          )}
        </label>
        <select
          className="key-kind-select"
          aria-label="按类型筛选"
          value={kind ?? ""}
          onChange={(event) => {
            setKind(event.target.value || null);
            setCursor(null);
          }}
        >
          <option value="">全部类型</option>
          {REDIS_KINDS.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
      </div>
      <div className="key-browser-tools">
        <div className="key-view-toggle" role="group" aria-label="显示方式">
          <button type="button" aria-pressed={grouped} title="按冒号分组" onClick={() => setGrouped(true)}>
            <FolderTree aria-hidden />
            <span className="visually-hidden">按冒号分组</span>
          </button>
          <button type="button" aria-pressed={!grouped} title="平铺" onClick={() => setGrouped(false)}>
            <List aria-hidden />
            <span className="visually-hidden">平铺</span>
          </button>
        </div>
        <Button
          variant="ghost"
          size="sm"
          icon={<RefreshCw />}
          aria-label="刷新键列表"
          title="从头重新扫描"
          busy={scan.isFetching && !scan.isFetchingNextPage}
          onClick={() => void scan.refetch()}
        />
        <Button variant="ghost" size="sm" icon={<Plus />} onClick={() => onNewKey(folderOfActive)}>
          新建键
        </Button>
      </div>

      <div
        ref={scrollRef}
        className="key-list"
        role="tree"
        aria-label={`db${db} 的键`}
        tabIndex={0}
        aria-activedescendant={rows.length ? `${listId}-${active}` : undefined}
        onKeyDown={onKeyDown}
      >
        {scan.isPending ? (
          <p className="key-list-status">正在扫描…</p>
        ) : scan.error ? (
          <p className="key-list-status is-error" role="alert">
            {errorMessage(scan.error)}
          </p>
        ) : rows.length === 0 ? (
          <p className="key-list-status">
            {filter || kind ? "没有符合条件的键" : "这个库是空的"}
            {scan.hasNextPage && "（还没扫完）"}
          </p>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => {
              const row = rows[item.index]!;
              const isSelected = row.kind === "key" && row.key.key.id === selected;
              return (
                <div
                  key={row.kind === "key" ? row.key.key.id : `folder:${row.path}`}
                  id={`${listId}-${item.index}`}
                  role="treeitem"
                  aria-label={
                    row.kind === "key"
                      ? `${row.key.key.name}（${row.key.kind}）`
                      : `${row.label}，${row.count.toLocaleString()} 个键`
                  }
                  aria-level={row.depth + 1}
                  aria-expanded={row.kind === "folder" ? row.expanded : undefined}
                  aria-selected={isSelected}
                  className={[
                    "key-row",
                    row.kind === "folder" ? "is-folder" : "",
                    isSelected ? "is-selected" : "",
                    item.index === active ? "is-active" : "",
                  ].join(" ")}
                  style={{
                    transform: `translateY(${item.start}px)`,
                    paddingLeft: 8 + row.depth * INDENT,
                  }}
                  title={row.kind === "key" ? row.key.key.name : `${row.path}:*`}
                  onClick={() => activate(row)}
                >
                  {row.kind === "folder" ? (
                    <>
                      <ChevronRight className="key-row-chevron" aria-hidden />
                      <span className="key-row-label">{row.label}</span>
                      <span className="key-row-count">{row.count.toLocaleString()}</span>
                    </>
                  ) : (
                    <>
                      <KindBadge kind={row.key.kind} />
                      <span className="key-row-label mono">{row.label}</span>
                      {row.key.ttlMs !== null && (
                        <span className="key-row-ttl" title={`还剩 ${formatTtl(row.key.ttlMs)}`}>
                          <Clock aria-hidden />
                          <span className="visually-hidden">会过期</span>
                        </span>
                      )}
                    </>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      <footer className="key-browser-footer">
        <span>
          已列出 {keys.length.toLocaleString()} 个
          {total !== undefined && !filter && !kind && ` / 共 ${total.toLocaleString()} 个`}
        </span>
        {scan.hasNextPage && (
          <Button size="sm" variant="ghost" busy={scan.isFetchingNextPage} onClick={() => void scan.fetchNextPage()}>
            继续扫描
          </Button>
        )}
      </footer>
    </section>
  );
}
