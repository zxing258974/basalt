// 对象树里 Redis 连接以下的节点:一个库一行,行尾是键数。
import { Database, KeyRound } from "lucide-react";
import { useRedisKeyspace } from "../../hooks/useCatalog";
import type { ConnectionView } from "../../lib/types";
import type { RedisDatabase } from "../../lib/redisTypes";
import { Children } from "./CatalogNodes";
import { redisKeysTab, useTabs } from "../../stores/tabs";
import { redisDbKey } from "./treeKeys";
import { useTreeMenu } from "./treeMenu";
import { TreeRow, TreeStatusRow } from "./TreeRow";

export function RedisDatabaseList({ connection, depth }: { connection: ConnectionView; depth: number }) {
  const query = useRedisKeyspace(connection.id, connection.isOpen);
  const keyspace = query.data;
  const databases = { ...query, data: keyspace?.databases };
  const hidden = keyspace ? keyspace.count - keyspace.databases.length : 0;
  return (
    <Children query={databases} depth={depth} empty="没有可访问的库">
      {(items) => (
        <>
          {items.map((database) => (
            <RedisDatabaseNode
              key={database.index}
              connection={connection}
              database={database}
              isDefault={database.index === keyspace?.defaultDb}
              depth={depth}
            />
          ))}
          {hidden > 0 && <TreeStatusRow depth={depth} status="empty" message={`另有 ${hidden} 个空库没有列出`} />}
        </>
      )}
    </Children>
  );
}

function RedisDatabaseNode(props: {
  connection: ConnectionView;
  database: RedisDatabase;
  isDefault: boolean;
  depth: number;
}) {
  const { connection, database, depth } = props;
  const openMenu = useTreeMenu();
  const name = `db${database.index}`;
  const expiring = database.expires > 0 ? `，其中 ${database.expires.toLocaleString()} 个会过期` : "";
  return (
    <TreeRow
      nodeKey={redisDbKey(connection.id, database.index)}
      depth={depth}
      label={props.isDefault ? <span className="redis-db-default">{name}</span> : name}
      accessibleName={props.isDefault ? `${name}（默认库）` : name}
      title={`${database.keys.toLocaleString()} 个键${expiring}${props.isDefault ? "。连接配置里的默认库" : ""}`}
      icon={<Database />}
      count={database.keys}
      onActivate={() => useTabs.getState().open(redisKeysTab(connection.id, database.index))}
      onContextMenu={(event) =>
        openMenu(event, [
          {
            label: "浏览键",
            icon: <KeyRound />,
            onSelect: () => useTabs.getState().open(redisKeysTab(connection.id, database.index)),
          },
        ])
      }
    />
  );
}
