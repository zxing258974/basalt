import { Copy, Eye, Table2 } from "lucide-react";
import { useMemo, useState } from "react";
import { useConnections, useTableDetails } from "../../hooks/useCatalog";
import { errorMessage, type TableRef } from "../../lib/types";
import { useInspector, type InspectorTab } from "../../stores/inspector";
import { useUi } from "../../stores/ui";
import { tableFromKey } from "../sidebar/treeKeys";
import { Button } from "../ui/Button";
import { formatProperty } from "./inspectorModel";

const TABS: { tab: InspectorTab; label: string }[] = [
  { tab: "properties", label: "属性" },
  { tab: "ddl", label: "DDL" },
];

/** 右侧信息面板:对象树里选中的表或视图的属性与建表语句。 */
export function InspectorPanel() {
  // 只订阅 key 字符串:选择器每次返回新对象会让 zustand 以为状态一直在变,无限重渲染。
  const selectedKey = useUi((s) => s.selectedKey);
  const selected = useMemo(() => tableFromKey(selectedKey), [selectedKey]);
  const tab = useInspector((s) => s.tab);
  const setTab = useInspector((s) => s.setTab);
  return (
    <aside className="inspector" aria-label="信息">
      <div className="inspector-tabs" role="tablist" aria-label="信息面板">
        {TABS.map((item) => (
          <button
            key={item.tab}
            type="button"
            role="tab"
            aria-selected={tab === item.tab}
            className={tab === item.tab ? "inspector-tab is-active" : "inspector-tab"}
            onClick={() => setTab(item.tab)}
          >
            {item.label}
          </button>
        ))}
      </div>
      {selected ? (
        <Details
          key={`${selected.connectionId}/${selected.database}/${selected.schema ?? ""}/${selected.table}`}
          connectionId={selected.connectionId}
          table={{ database: selected.database, schema: selected.schema, table: selected.table }}
          tab={tab}
        />
      ) : (
        <p className="inspector-empty">在左侧选中一张表或视图，这里显示它的属性和建表语句。</p>
      )}
    </aside>
  );
}

function Details({ connectionId, table, tab }: { connectionId: string; table: TableRef; tab: InspectorTab }) {
  const connection = useConnections().data?.find((c) => c.id === connectionId);
  const query = useTableDetails(connectionId, table);
  const [copied, setCopied] = useState(false);
  const details = query.data;
  const kind = details?.properties.find((p) => p.label === "类型")?.value;
  const isView = kind?.kind === "text" && kind.value.includes("视图");
  const where = [connection?.name, table.database, table.schema].filter(Boolean).join(" · ");
  return (
    <div className="inspector-body" role="tabpanel" aria-label={tab === "ddl" ? "DDL" : "属性"}>
      <header className="inspector-head">
        <span className="inspector-icon" aria-hidden>
          {isView ? <Eye /> : <Table2 />}
        </span>
        <div className="inspector-title">
          <h2 className="inspector-name selectable">{table.table}</h2>
          <p className="inspector-where">{where}</p>
        </div>
      </header>
      {query.isPending && <p className="inspector-empty">正在读取…</p>}
      {query.error && (
        <p className="inspector-error" role="alert">
          {errorMessage(query.error)}
        </p>
      )}
      {details && tab === "properties" && (
        <dl className="inspector-props">
          {details.properties.map((property) => (
            <div key={property.label} className="inspector-prop">
              <dt>{property.label}</dt>
              <dd className={property.value === null ? "is-missing" : "selectable"}>
                {formatProperty(property.value)}
              </dd>
            </div>
          ))}
        </dl>
      )}
      {details && tab === "ddl" && (
        <div className="inspector-ddl">
          <div className="inspector-ddl-head">
            <span className="inspector-ddl-note">
              {details.ddlGenerated ? "按表结构生成（PostgreSQL 没有现成的建表语句）" : "数据库给出的原文"}
            </span>
            <Button
              size="sm"
              variant="ghost"
              icon={<Copy />}
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(details.ddl)
                  .then(() => setCopied(true))
                  .catch((error: unknown) => console.warn("复制建表语句失败", error));
              }}
            >
              {copied ? "已复制" : "复制"}
            </Button>
          </div>
          <pre className="inspector-sql mono selectable" aria-label="建表语句">
            {details.ddl}
          </pre>
        </div>
      )}
    </div>
  );
}
