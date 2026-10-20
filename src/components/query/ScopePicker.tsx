import { useDatabases, useSchemas } from "../../hooks/useCatalog";
import type { QueryScope } from "../../lib/queryTypes";
import type { ConnectionView } from "../../lib/types";

interface ScopePickerProps {
  connection: ConnectionView;
  scope: QueryScope;
  disabled: boolean;
  onChange(scope: QueryScope): void;
}

const NONE = "";

/** 列表里还没有当前值(库刚被删、还在加载)时也要显示出来,免得下拉框悄悄跳到第一项。 */
function withCurrent(options: readonly string[] | undefined, current: string | null): string[] {
  const list = options ?? [];
  return current && !list.includes(current) ? [current, ...list] : [...list];
}

/** 标签页上方的库(PostgreSQL 还有 schema)选择。换了之后,下一条语句执行前切过去。 */
export function ScopePicker({ connection, scope, disabled, onChange }: ScopePickerProps) {
  const isPostgres = connection.driver === "postgres";
  const databases = useDatabases(connection.id, connection.isOpen);
  const schemas = useSchemas(connection.id, scope.database ?? "", connection.isOpen && isPostgres && !!scope.database);
  const databaseOptions = withCurrent(databases.data, scope.database);
  const schemaOptions = withCurrent(schemas.data, scope.schema);

  return (
    <div className="query-scope">
      <label className="query-scope-field">
        <span className="visually-hidden">库</span>
        <select
          aria-label="库"
          value={scope.database ?? NONE}
          disabled={disabled}
          onChange={(event) =>
            onChange({ database: event.target.value || null, schema: isPostgres ? null : scope.schema })
          }
        >
          {!scope.database && <option value={NONE}>（未选择库）</option>}
          {databaseOptions.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </label>
      {isPostgres && (
        <>
          <span className="query-scope-sep" aria-hidden>
            .
          </span>
          <label className="query-scope-field">
            <span className="visually-hidden">schema</span>
            <select
              aria-label="schema"
              value={scope.schema ?? NONE}
              disabled={disabled || !scope.database}
              onChange={(event) => onChange({ ...scope, schema: event.target.value || null })}
            >
              {!scope.schema && <option value={NONE}>（默认 schema）</option>}
              {schemaOptions.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </label>
        </>
      )}
    </div>
  );
}
