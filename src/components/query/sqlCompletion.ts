// 把 completion.ts 的纯逻辑接到 CodeMirror:表名、库 / schema 名来自对象树同一份缓存,
// 列名按需取(写到 `表.` 或语句里引用了这张表时才查)。
import type { Completion, CompletionContext, CompletionResult, CompletionSource } from "@codemirror/autocomplete";
import { syntaxTree } from "@codemirror/language";
import { completionTarget, referencedTables, resolveQualifier, type TableReference } from "./completion";
import { splitStatements, statementAtCursor, type SqlDialect } from "./splitStatements";

/** 补全要的元数据。namespace 是库(MySQL、SQLite)或 schema(PostgreSQL),null 表示当前的。 */
export interface CompletionProvider {
  tables(namespace: string | null): Promise<string[]>;
  columns(table: string, namespace: string | null): Promise<string[]>;
  namespaces(): Promise<string[]>;
}

const SIMPLE_NAME = /^[A-Za-z_][\w$]*$/;
const VALID_FOR = /^[\w$\u0080-\uffff]*$/;
const TYPING = /[\w$.`"\u0080-\uffff]*$/;

/** 不是普通标识符的名字(有空格、大写开头的 PG 名字等),补全时带上引号。 */
function quoteIfNeeded(name: string, dialect: SqlDialect): string {
  const plain = dialect === "postgres" ? /^[a-z_][a-z0-9_$]*$/.test(name) : SIMPLE_NAME.test(name);
  if (plain) return name;
  const quote = dialect === "mysql" ? "`" : '"';
  return `${quote}${name.replaceAll(quote, quote + quote)}${quote}`;
}

function options(names: string[], type: string, detail: string, dialect: SqlDialect, boost = 0): Completion[] {
  return names.map((name) => ({ label: name, apply: quoteIfNeeded(name, dialect), type, detail, boost }));
}

/** 光标落在字符串或注释里就不补。 */
function inLiteral(context: CompletionContext): boolean {
  const node = syntaxTree(context.state).resolveInner(context.pos, -1);
  return /String|Comment/.test(node.type.name);
}

async function columnsOf(provider: CompletionProvider, refs: TableReference[], dialect: SqlDialect) {
  const lists = await Promise.all(
    refs.map(async (ref) =>
      options(await provider.columns(ref.table, ref.namespace), "property", ref.table, dialect, 2),
    ),
  );
  return lists.flat();
}

export function schemaCompletion(provider: CompletionProvider, dialect: SqlDialect): CompletionSource {
  const namespaceLabel = dialect === "postgres" ? "schema" : "库";
  return async (context): Promise<CompletionResult | null> => {
    const before = context.state.sliceDoc(Math.max(0, context.pos - 300), context.pos);
    const target = completionTarget(before);
    if ((!target.qualifier && !target.word && !context.explicit) || inLiteral(context)) return null;
    const doc = context.state.doc.toString();
    const statement = statementAtCursor(splitStatements(doc, dialect), doc, context.pos);
    // 正在敲的这个(可能带点的)名字不算语句里引用的表:写到 `FROM crm.` 时 crm 是库名。
    const typing = context.pos - (TYPING.exec(before)?.[0].length ?? 0);
    const text = statement
      ? doc.slice(statement.from, Math.max(statement.from, typing)) + doc.slice(context.pos, statement.to)
      : "";
    const refs = referencedTables(text);
    try {
      let found: Completion[];
      if (target.qualifier) {
        const table = resolveQualifier(target.qualifier, refs);
        found = table
          ? await columnsOf(provider, [table], dialect)
          : options(await provider.tables(target.qualifier), "class", target.qualifier, dialect);
      } else {
        const [tables, namespaces, columns] = await Promise.all([
          provider.tables(null),
          provider.namespaces(),
          columnsOf(provider, refs, dialect),
        ]);
        found = [
          ...columns,
          ...options(tables, "class", "表", dialect, 1),
          ...options(namespaces, "namespace", namespaceLabel, dialect),
        ];
      }
      return { from: context.pos - target.word.length, options: found, validFor: VALID_FOR };
    } catch (error) {
      // 元数据读不到(连接断了、没权限)只是少了候选,不打扰用户。
      console.warn("读取补全用的元数据失败", error);
      return null;
    }
  };
}
