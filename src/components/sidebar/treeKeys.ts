// 对象树节点的 key:按层级拼路径,子节点总以「父 key + /」开头,
// 这样收起或清理某个连接下的全部节点只需前缀匹配。

export type GroupKind = "tables" | "views" | "queries";

const segment = (tag: string, name: string) => `${tag}:${encodeURIComponent(name)}`;

export const connectionKey = (id: string): string => segment("c", id);

/** 连接分组文件夹。不以 c: 开头,不会被当成某个连接的子节点。 */
export const folderKey = (group: string): string => segment("folder", group);

export const databaseKey = (id: string, database: string): string => `${connectionKey(id)}/${segment("d", database)}`;

export const schemaKey = (id: string, database: string, schema: string): string =>
  `${databaseKey(id, database)}/${segment("s", schema)}`;

/** 表 / 视图分组挂在 schema 下(PG)或直接挂在库下。 */
export function containerKey(id: string, database: string, schema: string | null): string {
  return schema === null ? databaseKey(id, database) : schemaKey(id, database, schema);
}

export const groupKey = (id: string, database: string, schema: string | null, group: GroupKind): string =>
  `${containerKey(id, database, schema)}/${segment("g", group)}`;

export const tableKey = (id: string, database: string, schema: string | null, table: string): string =>
  `${containerKey(id, database, schema)}/${segment("t", table)}`;

/** 保存的查询挂在「查询」分组下,和表一样按所在的库(PG 是 schema)区分。 */
export const savedQueryKey = (id: string, database: string, schema: string | null, name: string): string =>
  `${containerKey(id, database, schema)}/${segment("q", name)}`;

/** Redis 的库按库号。不用 d: 开头:那是 SQL 的库,⌘T 会照它开 SQL 查询。 */
export const redisDbKey = (id: string, db: number): string => `${connectionKey(id)}/${segment("r", String(db))}`;

/** Redis 库节点(或它下面的节点)的 key → 哪个连接的几号库;别的节点返回 null。 */
export function redisDbFromKey(key: string | null): { connectionId: string; db: number } | null {
  const connectionId = connectionIdFromKey(key);
  const found = key?.split("/").find((s) => s.startsWith("r:"));
  if (connectionId === null || found === undefined) return null;
  const db = Number(found.slice(2));
  return Number.isInteger(db) && db >= 0 ? { connectionId, db } : null;
}

/** 从节点 key 里读出它在哪个库、哪个 schema 下;连接节点本身两者都是 null。 */
export function locationFromKey(key: string | null): { database: string | null; schema: string | null } {
  const segments = key?.split("/") ?? [];
  const value = (tag: string) => {
    const found = segments.find((s) => s.startsWith(`${tag}:`));
    return found === undefined ? null : decodeURIComponent(found.slice(tag.length + 1));
  };
  return { database: value("d"), schema: value("s") };
}

/** 从任意节点 key 取回所属连接的 id;分组文件夹等非连接节点返回 null。 */
export function connectionIdFromKey(key: string | null): string | null {
  const first = key?.split("/")[0];
  if (!first?.startsWith("c:")) return null;
  return decodeURIComponent(first.slice(2));
}

/** 表或视图节点的 key → 它是哪个连接、哪个库(schema)里的哪张表;别的节点返回 null。 */
export function tableFromKey(
  key: string | null,
): { connectionId: string; database: string; schema: string | null; table: string } | null {
  const last = key?.split("/").at(-1);
  const connectionId = connectionIdFromKey(key);
  const { database, schema } = locationFromKey(key);
  if (!last?.startsWith("t:") || connectionId === null || database === null) return null;
  return { connectionId, database, schema, table: decodeURIComponent(last.slice(2)) };
}
