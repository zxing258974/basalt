import { describe, expect, it } from "vitest";
import { formatEdit, formatSql } from "./formatSql";

describe("formatSql", () => {
  it("puts each clause on its own line, keeping MySQL's letter case", async () => {
    expect(await formatSql("select id, name from customers where vip = 1 order by id", "mysql")).toBe(
      ["select", "  id,", "  name", "from", "  customers", "where", "  vip = 1", "order by", "  id"].join("\n"),
    );
  });

  it("upper-cases keywords where names are not case-sensitive", async () => {
    expect(await formatSql("select x::text from t", "postgres")).toBe("SELECT\n  x::TEXT\nFROM\n  t");
    expect(await formatSql("select a from t", "sqlite")).toBe("SELECT\n  a\nFROM\n  t");
    const body = "$$ begin return 1; end $$";
    expect(await formatSql(`create function f() returns int as ${body} language plpgsql`, "postgres")).toContain(body);
  });

  it("says where it got lost", async () => {
    await expect(formatSql("select (( from t", "mysql")).rejects.toThrow(/^第 1 行第 \d+ 列附近读不懂$/);
  });
});

describe("formatEdit", () => {
  const text = "select 1;\n\nselect a, b from t;\n";

  it("formats everything when nothing is selected, keeping the cursor on the same word", async () => {
    const at = text.indexOf("from");
    expect(await formatEdit(text, { from: at + 2, to: at + 2 }, "mysql")).toEqual({
      kind: "edit",
      from: 0,
      to: text.length,
      text: "select\n  1;\n\nselect\n  a,\n  b\nfrom\n  t;\n",
      cursor: "select\n  1;\n\nselect\n  a,\n  b\nfr".length,
    });
  });

  it("formats only the selection and keeps the blank space around it", async () => {
    const from = text.indexOf("select a");
    expect(await formatEdit(text, { from: text.length, to: from }, "mysql")).toEqual({
      kind: "edit",
      from,
      to: text.length,
      text: "select\n  a,\n  b\nfrom\n  t;\n",
      cursor: null,
    });
  });

  it("refuses scripts with DELIMITER anywhere, and selections cut through strings or comments", async () => {
    const script = "DELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; END //";
    const body = script.indexOf("CREATE");
    await expect(formatEdit(script, { from: body, to: script.length }, "mysql")).rejects.toThrow("DELIMITER");
    const commented = "select a -- note\n, b from t";
    await expect(
      formatEdit(commented, { from: commented.indexOf("note"), to: commented.length }, "mysql"),
    ).rejects.toThrow("中间开始或结束");
  });

  it("reports where it got lost in the whole editor", async () => {
    const doc = "select 1;\n\nselect (( from t";
    const from = doc.indexOf("select ((");
    await expect(formatEdit(doc, { from, to: doc.length }, "mysql")).rejects.toThrow(/^第 3 行第 \d+ 列附近读不懂$/);
  });

  it("tells blank and already formatted text apart", async () => {
    expect(await formatEdit("  \n", { from: 0, to: 0 }, "mysql")).toEqual({ kind: "empty", selection: false });
    expect(await formatEdit("select 1;   ", { from: 9, to: 12 }, "mysql")).toEqual({ kind: "empty", selection: true });
    expect(await formatEdit("select\n  1", { from: 0, to: 0 }, "mysql")).toEqual({ kind: "unchanged" });
  });
});
