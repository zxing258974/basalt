import { describe, expect, it } from "vitest";
import { guardFormatted, insideLiteral, sqlTokens } from "./formatGuard";
import { formatSql } from "./formatSql";

const texts = (sql: string, dialect: "mysql" | "postgres" | "sqlite") => sqlTokens(sql, dialect).map((t) => t.text);

describe("sqlTokens", () => {
  it("keeps strings, quoted names and comments whole, per dialect", () => {
    expect(texts("select 'it\\'s' # x\n", "mysql")).toEqual(["select", "'it\\'s'", "# x"]);
    expect(texts("select 'C:\\' , E'a\\'b'", "postgres")).toEqual(["select", "'C:\\'", ",", "E", "'a\\'b'"]);
    expect(texts('select $f$ a; b $f$, "Mixed"', "postgres")).toEqual(["select", "$f$ a; b $f$", ",", '"Mixed"']);
    expect(texts("select [a b] from t -- c", "sqlite")).toEqual(["select", "[a b]", "from", "t", "-- c"]);
    // MySQL 的 --1 不是注释。
    expect(texts("select 1 --1", "mysql")).toEqual(["select", "1", "-", "-", "1"]);
    expect(texts("/* a /* b */ c */ x", "postgres")).toEqual(["/* a /* b */ c */", "x"]);
  });

  it("knows when a position is inside a string or comment", () => {
    const tokens = sqlTokens("select 'a from b', c -- note", "mysql");
    expect(insideLiteral(tokens, 10)).toBe(true);
    expect(insideLiteral(tokens, 7)).toBe(false);
    expect(insideLiteral(tokens, 24)).toBe(true);
  });
});

describe("guardFormatted", () => {
  it("refuses output whose words differ", () => {
    expect(guardFormatted("select a b", "select a, b", "mysql", false)).toBeNull();
    expect(guardFormatted("select Text", "SELECT TEXT", "mysql", false)).toBeNull();
    expect(guardFormatted("select Text", "SELECT TEXT", "postgres", true)?.text).toBe("SELECT TEXT");
  });

  it("glues back what spacing would change", () => {
    expect(guardFormatted("select sysdate()", "select\n  sysdate ()", "mysql", false)?.text).toBe(
      "select\n  sysdate()",
    );
    expect(guardFormatted("select a===b", "SELECT\n  a = = = b", "postgres", true)?.text).toBe("SELECT\n  a === b");
    expect(guardFormatted("select 'a'\n'b'", "SELECT\n  'a' 'b'", "postgres", true)?.text).toBe("SELECT\n  'a'\n'b'");
    // 代码被挪到行注释后面,就成了注释的一部分:词数对不上,不换。
    expect(guardFormatted("select 1 -- x\n, 2", "select\n  1 -- x, 2", "mysql", false)).toBeNull();
  });
});

describe("formatting keeps the meaning", () => {
  it.each([
    ["mysql", "select Text.id from t as Text join bit on bit.id = Text.id"],
    ["mysql", "select @a:=@a+1, x'0f', b'1', `order` from t # c\nwhere a ->> '$.x' = 1"],
    ["mysql", "select /*! STRAIGHT_JOIN */ 'it\\'s' from t"],
    ["mariadb", "select sysdate(), last_value(a) over () from t"],
    ["postgres", "select x::text, d @> '{1}', E'a\\'b', $$ x; y $$ from t where y ilike 'a%'"],
    ["postgres", "select a===b from t"],
    ["sqlite", "select [a b], ?1, :n from t"],
  ] as const)("%s: %s", async (driver, sql) => {
    const formatted = await formatSql(sql, driver);
    const dialect = driver === "mariadb" ? "mysql" : driver;
    const ignoreCase = driver === "postgres" || driver === "sqlite";
    const norm = (s: string) =>
      sqlTokens(s, dialect).map((t) => (ignoreCase && t.kind === "word" ? t.text.toLowerCase() : t.text));
    expect(norm(formatted)).toEqual(norm(sql));
    if (driver === "mariadb") expect(formatted).toContain("sysdate()");
    if (driver === "mysql") expect(formatted).not.toMatch(/\bTEXT\b|\bBIT\b/);
  });
});
