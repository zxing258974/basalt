// mock 后端的表维护:语句、消息、前后大小都照各家真实的样子给,方便在浏览器里看结果对话框。
import type { CommandError, DriverKind, Maintenance, MaintenanceOutcome, TableRef } from "../types";

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
