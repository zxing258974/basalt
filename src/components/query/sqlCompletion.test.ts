import { CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { MySQL, PostgreSQL, sql } from "@codemirror/lang-sql";
import { ensureSyntaxTree } from "@codemirror/language";
import { EditorState } from "@codemirror/state";
import { describe, expect, it, vi } from "vitest";
import { schemaCompletion, type CompletionProvider } from "./sqlCompletion";

function provider(overrides: Partial<CompletionProvider> = {}): CompletionProvider {
  return {
    tables: vi.fn((namespace: string | null) =>
      Promise.resolve(namespace === "crm" ? ["leads"] : ["orders", "order items"]),
    ),
    columns: vi.fn((table: string) => Promise.resolve(table === "orders" ? ["id", "customer_id"] : ["Name"])),
    namespaces: vi.fn(() => Promise.resolve(["shop", "crm"])),
    ...overrides,
  };
}

/** `|` 是光标位置。 */
async function complete(
  textWithCursor: string,
  source = provider(),
  dialect: "mysql" | "postgres" = "mysql",
  explicit = false,
): Promise<CompletionResult | null> {
  const pos = textWithCursor.indexOf("|");
  const doc = textWithCursor.replace("|", "");
  const state = EditorState.create({ doc, extensions: [sql({ dialect: dialect === "mysql" ? MySQL : PostgreSQL })] });
  ensureSyntaxTree(state, doc.length, 1000);
  const result = await schemaCompletion(source, dialect)(new CompletionContext(state, pos, explicit));
  return result;
}

const labels = (result: CompletionResult | null) => result?.options.map((o) => o.label) ?? [];

describe("schemaCompletion", () => {
  it("offers columns of referenced tables, then tables and databases", async () => {
    const result = await complete("SELECT cu| FROM orders o");
    expect(result?.from).toBe("SELECT ".length);
    expect(labels(result)).toEqual(["id", "customer_id", "orders", "order items", "shop", "crm"]);
    const quoted = result?.options.find((o) => o.label === "order items");
    expect(quoted?.apply).toBe("`order items`");
    expect(result?.options.find((o) => o.label === "shop")?.detail).toBe("库");
  });

  it("completes columns after an alias and tables after a database name", async () => {
    const source = provider();
    expect(labels(await complete("SELECT o.| FROM orders o", source))).toEqual(["id", "customer_id"]);
    expect(source.columns).toHaveBeenCalledWith("orders", null);
    expect(labels(await complete("SELECT * FROM crm.|", source))).toEqual(["leads"]);
    expect(source.tables).toHaveBeenCalledWith("crm");
  });

  it("quotes PostgreSQL names that are not plain lower-case identifiers", async () => {
    const result = await complete('SELECT u.| FROM "Users" u', provider(), "postgres");
    expect(result?.options[0]).toMatchObject({ label: "Name", apply: '"Name"' });
    const namespaces = await complete("SELECT x|", provider(), "postgres");
    expect(namespaces?.options.find((o) => o.label === "crm")?.detail).toBe("schema");
  });

  it("stays quiet inside strings and comments, and when nothing is typed", async () => {
    expect(await complete("SELECT 'ord|'")).toBeNull();
    expect(await complete("-- ord|")).toBeNull();
    expect(await complete("SELECT |")).toBeNull();
    expect(labels(await complete("SELECT |", provider(), "mysql", true))).toContain("orders");
  });

  it("gives up quietly when metadata cannot be read", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const failing = provider({ tables: () => Promise.reject(new Error("断开了")) });
    expect(await complete("SELECT ord|", failing)).toBeNull();
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});
