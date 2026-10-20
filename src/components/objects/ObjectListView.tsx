import { ArrowDown, ArrowUp, Eye, RefreshCw, Search, Table2 } from "lucide-react";
import { useMemo, useState } from "react";
import { useConnections, useTables } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { formatBytes, formatCount } from "../../lib/format";
import { errorMessage, type TableInfo, type TableKind } from "../../lib/types";
import { tableDataTab, useTabs, type ObjectsTab } from "../../stores/tabs";
import { DriverBadge } from "../connection/DriverBadge";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import {
  filterTables,
  sortTables,
  visibleColumns,
  type KindFilter,
  type OptionalColumn,
  type SortKey,
  type TableSort,
} from "./objectList";

const KIND_LABEL: Record<TableKind, string> = {
  table: "表",
  view: "视图",
  materializedView: "物化视图",
  foreignTable: "外部表",
};

interface ColumnSpec {
  label: string;
  sortKey?: SortKey;
  numeric?: boolean;
  render(table: TableInfo): string;
}

const COLUMNS: Record<OptionalColumn, ColumnSpec> = {
  rows: { label: "行数(估)", sortKey: "rows", numeric: true, render: (t) => formatCount(t.rows) },
  dataBytes: { label: "数据大小", sortKey: "dataBytes", numeric: true, render: (t) => formatBytes(t.dataBytes) },
  indexBytes: { label: "索引大小", sortKey: "indexBytes", numeric: true, render: (t) => formatBytes(t.indexBytes) },
  engine: { label: "引擎", render: (t) => t.engine ?? "" },
  updatedAt: { label: "修改时间", sortKey: "updatedAt", render: (t) => t.updatedAt ?? "" },
  comment: { label: "注释", render: (t) => t.comment ?? "" },
};

// 数据没到时用同一个空数组,免得下面的 useMemo 每次渲染都重算。
const NO_TABLES: TableInfo[] = [];

const FILTERS: { value: KindFilter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "tables", label: "表" },
  { value: "views", label: "视图" },
];

export function ObjectListView({ tab }: { tab: ObjectsTab }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const { openWithFeedback } = useConnectionActions();
  const query = useTables(tab.connectionId, tab.database, tab.schema, Boolean(connection?.isOpen));
  const [kind, setKind] = useState<KindFilter>("all");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<TableSort>({ key: "name", direction: "asc" });

  const all = query.data ?? NO_TABLES;
  const columns = useMemo(() => visibleColumns(all), [all]);
  const rows = useMemo(() => sortTables(filterTables(all, { kind, search }), sort), [all, kind, search, sort]);
  const tableCount = all.filter((t) => t.kind === "table").length;
  const openTab = useTabs((s) => s.open);
  const openData = (table: TableInfo) => openTab(tableDataTab(tab.connectionId, tab.database, tab.schema, table.name));

  const toggleSort = (key: SortKey) =>
    setSort((current) =>
      current.key === key
        ? { key, direction: current.direction === "asc" ? "desc" : "asc" }
        : { key, direction: key === "name" ? "asc" : "desc" },
    );

  return (
    <section className="objects" aria-label={`${tab.title} 的对象`}>
      <header className="objects-header">
        <div className="objects-heading">
          {connection && <DriverBadge driver={connection.driver} />}
          <span className="objects-crumb">{connection?.name ?? "已关闭的连接"}</span>
          <span className="objects-crumb-sep">/</span>
          <h1 className="objects-title">{tab.title}</h1>
          <span className="objects-summary">
            {tableCount} 张表 · {all.length - tableCount} 个视图
          </span>
        </div>
        <div className="objects-toolbar">
          <div className="segmented" role="radiogroup" aria-label="对象类型">
            {FILTERS.map((f) => (
              <button
                key={f.value}
                role="radio"
                aria-checked={kind === f.value}
                className={kind === f.value ? "segmented-item is-active" : "segmented-item"}
                onClick={() => setKind(f.value)}
              >
                {f.label}
              </button>
            ))}
          </div>
          <label className="search-box">
            <Search aria-hidden />
            <input
              value={search}
              placeholder="按名称或注释筛选"
              aria-label="筛选对象"
              onChange={(e) => setSearch(e.target.value)}
              spellCheck={false}
            />
          </label>
          <Button
            variant="ghost"
            size="sm"
            icon={<RefreshCw />}
            aria-label="刷新"
            busy={query.isFetching}
            onClick={() => void query.refetch()}
          />
        </div>
      </header>

      {!connection?.isOpen ? (
        <EmptyState
          title={connection ? "连接已关闭" : "这个连接已经不存在"}
          action={
            connection && (
              <Button variant="primary" onClick={() => void openWithFeedback(connection)}>
                重新打开
              </Button>
            )
          }
        />
      ) : query.error ? (
        <EmptyState title="读取对象失败" detail={errorMessage(query.error)} tone="error" />
      ) : query.isPending ? (
        <EmptyState title="正在读取对象…" />
      ) : rows.length === 0 ? (
        <EmptyState title={all.length === 0 ? "这里还没有任何表或视图" : "没有符合筛选条件的对象"} />
      ) : (
        <div className="objects-scroll">
          <table className="objects-table">
            <thead>
              <tr>
                <SortableHeader label="名称" sortKey="name" sort={sort} onSort={toggleSort} />
                <th className="col-kind">类型</th>
                {columns.map((column) => {
                  const spec = COLUMNS[column];
                  return spec.sortKey ? (
                    <SortableHeader
                      key={column}
                      label={spec.label}
                      sortKey={spec.sortKey}
                      sort={sort}
                      numeric={spec.numeric}
                      onSort={toggleSort}
                    />
                  ) : (
                    <th key={column}>{spec.label}</th>
                  );
                })}
              </tr>
            </thead>
            <tbody>
              {rows.map((table) => (
                <tr
                  key={table.name}
                  tabIndex={0}
                  title="双击打开数据"
                  onDoubleClick={() => openData(table)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") openData(table);
                  }}
                >
                  <td className="col-name">
                    <span className="col-name-inner">
                      {table.kind === "table" ? <Table2 aria-hidden /> : <Eye aria-hidden />}
                      <span className="selectable">{table.name}</span>
                    </span>
                  </td>
                  <td className="col-kind">{KIND_LABEL[table.kind]}</td>
                  {columns.map((column) => (
                    <td key={column} className={COLUMNS[column].numeric ? "is-numeric mono" : undefined}>
                      <span className={column === "comment" ? "selectable" : undefined}>
                        {COLUMNS[column].render(table)}
                      </span>
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function SortableHeader({
  label,
  sortKey,
  sort,
  numeric,
  onSort,
}: {
  label: string;
  sortKey: SortKey;
  sort: TableSort;
  numeric?: boolean;
  onSort(key: SortKey): void;
}) {
  const active = sort.key === sortKey;
  const ariaSort = active ? (sort.direction === "asc" ? "ascending" : "descending") : "none";
  return (
    <th aria-sort={ariaSort} className={numeric ? "is-numeric" : undefined}>
      <button className={active ? "sort-button is-active" : "sort-button"} onClick={() => onSort(sortKey)}>
        {label}
        {active && (sort.direction === "asc" ? <ArrowUp aria-hidden /> : <ArrowDown aria-hidden />)}
      </button>
    </th>
  );
}
