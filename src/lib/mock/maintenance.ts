// mock 后端的表维护:语句、消息、前后大小都照各家真实的样子给,方便在浏览器里看结果对话框。
// 也给出示例库里外键的引用关系,清空、截断的确认框据此提醒会波及哪些表。
import type {
  ClearMode,
  CommandError,
  DriverKind,
  Maintenance,
  MaintenanceOutcome,
  ReferencingKey,
  TableRef,
} from "../types";

const AVAILABLE: Record<DriverKind, Maintenance[]> = {
  mysql: ["analyze", "check", "optimize", "repair"],
  mariadb: ["analyze", "check", "optimize", "repair"],
  postgres: ["vacuum", "analyze", "reindex"],
  sqlite: ["analyze", "vacuum"],
};

const KEYWORD: Record<Maintenance, string> = {
  analyze: "ANALYZE",
  check: "CHECK",
  optimize: "OPTIMIZE",
  repair: "REPAIR",
  vacuum: "VACUUM",
  reindex: "REINDEX",
};

const isMysql = (driver: DriverKind) => driver === "mysql" || driver === "mariadb";

function quoted(driver: DriverKind, table: TableRef): string {
  const q = (name: string) => (isMysql(driver) ? `\`${name}\`` : `"${name}"`);
  const first = driver === "postgres" ? (table.schema ?? "public") : table.database;
  return `${q(first)}.${q(table.table)}`;
}

/** InnoDB 上 OPTIMIZE 是重建表、REPAIR 不支持,和真库返回的一样。 */
function mysqlMessages(operation: Maintenance) {
  if (operation === "optimize")
    return [
      { level: "note", text: "Table does not support optimize, doing recreate + analyze instead" },
      { level: "status", text: "OK" },
    ];
  if (operation === "repair")
    return [{ level: "note", text: "The storage engine for the table doesn't support repair" }];
  return [{ level: "status", text: "OK" }];
}

function sqlOf(driver: DriverKind, table: TableRef, operation: Maintenance): string {
  const name = quoted(driver, table);
  if (isMysql(driver)) return `${KEYWORD[operation]} TABLE ${name}`;
  if (operation === "reindex") return `REINDEX TABLE ${name}`;
  if (driver === "sqlite" && operation === "vacuum") return `VACUUM "${table.database}"`;
  return `${KEYWORD[operation]} ${name}`;
}

export function mockMaintenance(driver: DriverKind, table: TableRef, operation: Maintenance): MaintenanceOutcome {
  if (!AVAILABLE[driver].includes(operation)) {
    const error: CommandError = { code: "unsupported", message: `没有 ${KEYWORD[operation]} 这项维护`, fields: [] };
    throw error;
  }
  const measured = !isMysql(driver) && (operation === "vacuum" || operation === "reindex");
  return {
    sql: sqlOf(driver, table, operation),
    messages: isMysql(driver) ? mysqlMessages(operation) : [],
    size: measured ? { before: 1_843_200, after: 614_400 } : null,
    elapsedMs: operation === "analyze" ? 18 : 342,
  };
}

/** 示例 shop 库:订单引用客户(删客户连带删订单),订单明细引用商品(有明细时不能删商品)。 */
export function mockReferences(table: TableRef): ReferencingKey[] {
  const from = (name: string, key: string, onDelete: ReferencingKey["onDelete"]): ReferencingKey => ({
    table: { ...table, table: name },
    name: key,
    onDelete,
  });
  if (table.table === "customers") return [from("orders", "fk_orders_customer", "cascade")];
  if (table.table === "products") return [from("order_items", "fk_items_product", "restrict")];
  return [];
}

/**
 * 像真库那样按外键拒绝:MySQL、PostgreSQL 的截断只要被引用就拒绝;清空(以及 SQLite 的截断,它也是 DELETE)
 * 碰上 RESTRICT / NO ACTION 的引用就失败(示例数据里那边总有引用它的行)。
 */
export function refuseByForeignKeys(driver: DriverKind, table: TableRef, mode: ClearMode): void {
  const keys = mockReferences(table);
  const fail = (message: string): never => {
    const error: CommandError = { code: "database", message, fields: [] };
    throw error;
  };
  if (mode === "truncate" && driver !== "sqlite") {
    const key = keys[0];
    if (!key) return;
    fail(
      isMysql(driver)
        ? `Cannot truncate a table referenced in a foreign key constraint (\`${key.table.database}\`.\`${key.table.table}\`, CONSTRAINT \`${key.name ?? ""}\`)`
        : `cannot truncate a table referenced in a foreign key constraint`,
    );
  }
  const blocking = keys.find((k) => k.onDelete === "restrict" || k.onDelete === "noAction");
  if (!blocking) return;
  fail(
    driver === "sqlite"
      ? "FOREIGN KEY constraint failed"
      : `Cannot delete or update a parent row: a foreign key constraint fails (\`${blocking.table.table}\`)`,
  );
}
