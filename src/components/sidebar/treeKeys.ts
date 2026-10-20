// 对象树节点的 key:按层级拼路径,子节点总以「父 key + /」开头,
// 这样收起或清理某个连接下的全部节点只需前缀匹配。

export type GroupKind = "tables" | "views";

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

/** 从任意节点 key 取回所属连接的 id;分组文件夹等非连接节点返回 null。 */
export function connectionIdFromKey(key: string | null): string | null {
  const first = key?.split("/")[0];
  if (!first?.startsWith("c:")) return null;
  return decodeURIComponent(first.slice(2));
}
