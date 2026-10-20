// 对象树里连接以下的各层节点。每层只在展开时才去取数据。
import { Database, Eye, Layers, RefreshCw, Rows3, Table2, TableProperties } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { catalogKeys, useDatabases, useSchemas, useTables } from "../../hooks/useCatalog";
import { errorMessage, type ConnectionView, type TableInfo } from "../../lib/types";
import { objectsTab, tableDataTab, useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { databaseKey, groupKey, schemaKey, tableKey, type GroupKind } from "./treeKeys";
import { useTreeMenu } from "./treeMenu";
import { TreeRow, TreeStatusRow } from "./TreeRow";
import { designTableItem, newTableItem } from "../design/designMenu";

interface QueryLike<T> {
  data?: T;
  isPending: boolean;
  error: unknown;
  refetch(): unknown;
}

/** 统一处理子节点的加载中、出错、为空三种状态。 */
function Children<T>({
  query,
  depth,
  empty,
  children,
}: {
  query: QueryLike<T[]>;
  depth: number;
  empty: string;
  children(items: T[]): React.ReactNode;
}) {
  if (query.isPending) return <TreeStatusRow depth={depth} status="loading" message="加载中…" />;
  if (query.error) {
    return (
      <TreeStatusRow
        depth={depth}
        status="error"
        message={errorMessage(query.error)}
        onRetry={() => void query.refetch()}
      />
    );
  }
  const items = query.data ?? [];
  if (items.length === 0) return <TreeStatusRow depth={depth} status="empty" message={empty} />;
  return <>{children(items)}</>;
}

function useOpenObjects(connection: ConnectionView) {
  const openTab = useTabs((s) => s.open);
  return (database: string, schema: string | null) => openTab(objectsTab(connection.id, database, schema));
}

export function DatabaseList({ connection, depth }: { connection: ConnectionView; depth: number }) {
  const query = useDatabases(connection.id, connection.isOpen);
  return (
    <Children query={query} depth={depth} empty="没有可访问的数据库">
      {(databases) =>
        databases.map((database) => (
          <DatabaseNode key={database} connection={connection} database={database} depth={depth} />
        ))
      }
    </Children>
  );
}

function DatabaseNode({
  connection,
  database,
  depth,
}: {
  connection: ConnectionView;
  database: string;
  depth: number;
}) {
  const openMenu = useTreeMenu();
  const queryClient = useQueryClient();
  const openObjects = useOpenObjects(connection);
  const isPostgres = connection.driver === "postgres";
  const key = databaseKey(connection.id, database);

  return (
    <TreeRow
      nodeKey={key}
      depth={depth}
      label={database}
      icon={<Database />}
      expandable
      onActivate={
        isPostgres
          ? undefined
          : () => {
              useUi.getState().setExpanded(key, true);
              openObjects(database, null);
            }
      }
      onContextMenu={(event) =>
        openMenu(event, [
          ...(isPostgres
            ? []
            : [{ label: "打开对象列表", icon: <TableProperties />, onSelect: () => openObjects(database, null) }]),
          {
            label: "刷新",
            icon: <RefreshCw />,
            onSelect: () =>
              void queryClient.invalidateQueries({ queryKey: [...catalogKeys.connection(connection.id)] }),
          },
          ...(isPostgres ? [] : ["separator" as const, newTableItem(connection.id, database, null)]),
        ])
      }
    >
      {isPostgres ? (
        <SchemaList connection={connection} database={database} depth={depth + 1} />
      ) : (
        <ObjectGroups connection={connection} database={database} schema={null} depth={depth + 1} />
      )}
    </TreeRow>
  );
}

function SchemaList({ connection, database, depth }: { connection: ConnectionView; database: string; depth: number }) {
  const query = useSchemas(connection.id, database, true);
  const openObjects = useOpenObjects(connection);
  const openMenu = useTreeMenu();
  return (
    <Children query={query} depth={depth} empty="没有 schema">
      {(schemas) =>
        schemas.map((schema) => {
          const key = schemaKey(connection.id, database, schema);
          return (
            <TreeRow
              key={schema}
              nodeKey={key}
              depth={depth}
              label={schema}
              icon={<Layers />}
              expandable
              onActivate={() => {
                useUi.getState().setExpanded(key, true);
                openObjects(database, schema);
              }}
              onContextMenu={(event) =>
                openMenu(event, [
                  { label: "打开对象列表", icon: <TableProperties />, onSelect: () => openObjects(database, schema) },
                  "separator",
                  newTableItem(connection.id, database, schema),
                ])
              }
            >
              <ObjectGroups connection={connection} database={database} schema={schema} depth={depth + 1} />
            </TreeRow>
          );
        })
      }
    </Children>
  );
}

const GROUPS: { kind: GroupKind; label: string; matches(table: TableInfo): boolean }[] = [
  { kind: "tables", label: "表", matches: (t) => t.kind === "table" },
  { kind: "views", label: "视图", matches: (t) => t.kind !== "table" },
];

function ObjectGroups({
  connection,
  database,
  schema,
  depth,
}: {
  connection: ConnectionView;
  database: string;
  schema: string | null;
  depth: number;
}) {
  const query = useTables(connection.id, database, schema, true);
  const openObjects = useOpenObjects(connection);
  const openTab = useTabs((s) => s.open);
  const openMenu = useTreeMenu();
  const openData = (table: string) => openTab(tableDataTab(connection.id, database, schema, table));
  if (query.isPending || query.error) {
    return (
      <Children query={query} depth={depth} empty="">
        {() => null}
      </Children>
    );
  }
  const tables = query.data ?? [];
  return (
    <>
      {GROUPS.map((group) => {
        const members = tables.filter(group.matches);
        return (
          <TreeRow
            key={group.kind}
            nodeKey={groupKey(connection.id, database, schema, group.kind)}
            depth={depth}
            label={group.label}
            icon={group.kind === "tables" ? <Table2 /> : <Eye />}
            count={members.length}
            expandable={members.length > 0}
            onActivate={members.length > 0 ? undefined : () => openObjects(database, schema)}
          >
            {members.map((table) => (
              <TreeRow
                key={table.name}
                nodeKey={tableKey(connection.id, database, schema, table.name)}
                depth={depth + 1}
                label={table.name}
                title={table.comment ?? undefined}
                icon={table.kind === "table" ? <Table2 /> : <Eye />}
                onActivate={() => openData(table.name)}
                onContextMenu={(event) =>
                  openMenu(event, [
                    { label: "打开数据", icon: <Rows3 />, shortcut: "↩", onSelect: () => openData(table.name) },
                    { label: "打开对象列表", icon: <TableProperties />, onSelect: () => openObjects(database, schema) },
                    ...(table.kind === "table"
                      ? ["separator" as const, designTableItem(connection.id, database, schema, table.name)]
                      : []),
                  ])
                }
              />
            ))}
          </TreeRow>
        );
      })}
    </>
  );
}
