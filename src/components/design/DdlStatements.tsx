import type { ExecMode } from "../../lib/designTypes";

const VERB_TONE: Record<string, string> = {
  CREATE: "is-insert",
  ALTER: "is-update",
  COMMENT: "is-update",
  RENAME: "is-update",
  DROP: "is-delete",
};

/** 一条 DDL:开头的动词按语义上色(建=绿、改=余烬、删=红),多行原样保留缩进。 */
function Statement({ sql }: { sql: string }) {
  const match = /^(\S+)([\s\S]*)$/.exec(sql);
  const verb = match?.[1] ?? "";
  return (
    <code className="sql-code selectable">
      <span className={`sql-verb ${VERB_TONE[verb.toUpperCase()] ?? ""}`}>{verb}</span>
      {match?.[2] ?? ""}
    </code>
  );
}

export function DdlStatements({ statements }: { statements: string[] }) {
  return (
    <ol className="sql-list ddl-list" aria-label="将要执行的语句">
      {statements.map((sql, index) => (
        <li key={index} className="sql-item">
          <span className="sql-number mono">{index + 1}</span>
          <Statement sql={sql} />
        </li>
      ))}
    </ol>
  );
}

/** 各种执行方式该怎么跟用户说。 */
export function modeDescription(mode: ExecMode, count: number): string {
  switch (mode) {
    case "autocommit":
      return `MySQL 的 DDL 会立即生效、无法回滚：将逐条执行下面 ${count} 条语句，中途失败时会说明哪几条已经生效。`;
    case "rebuild":
      return `SQLite 不能直接这样修改，要重建整张表：临时关闭外键约束，在一个事务里执行下面 ${count} 条语句，提交前做外键检查，任何一步失败都整体回滚。`;
    case "transaction":
      return `将在一个事务里执行下面 ${count} 条语句；任何一条失败都会整体回滚。`;
  }
}
