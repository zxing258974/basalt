import { describe, expect, it } from "vitest";
import type { KeySummary } from "../../lib/redisTypes";
import {
  MAX_TTL_SECONDS,
  browserRows,
  formatLength,
  formatTtl,
  mergeKeys,
  parseTtl,
  scanPattern,
} from "./redisKeyModel";

const key = (name: string, kind = "string", binary = false): KeySummary => ({
  key: { id: `s:${name}`, name, binary },
  kind,
  ttlMs: null,
});

describe("scanPattern", () => {
  it("finds names containing plain text and keeps globs as written", () => {
    expect(scanPattern("")).toBe("*");
    expect(scanPattern("  ")).toBe("*");
    expect(scanPattern("session")).toBe("*session*");
    expect(scanPattern("user:*")).toBe("user:*");
    expect(scanPattern("user:100?")).toBe("user:100?");
    expect(scanPattern("[ab]*")).toBe("[ab]*");
    // 反斜杠、单独的 ] 是普通的字,照样按「含有」找。
    expect(scanPattern("a\\b")).toBe("*a\\\\b*");
    expect(scanPattern("arr]")).toBe("*arr]*");
  });
});

describe("browserRows", () => {
  const keys = [key("user:10"), key("user:2"), key("user:2:cart", "list"), key("counter"), key("product:1")];

  it("lists keys flat, numbers sorted by size", () => {
    expect(browserRows(keys, false, new Set()).map((r) => r.label)).toEqual([
      "counter",
      "product:1",
      "user:2",
      "user:2:cart",
      "user:10",
    ]);
  });

  it("groups by colon with folders first and only expanded folders opened", () => {
    const closed = browserRows(keys, true, new Set());
    expect(closed.map((r) => `${r.kind}:${r.label}:${r.depth}`)).toEqual([
      "folder:product:0",
      "folder:user:0",
      "key:counter:0",
    ]);
    expect(closed[1]).toMatchObject({ kind: "folder", path: "user", count: 3, expanded: false });

    const open = browserRows(keys, true, new Set(["user", "user:2"]));
    expect(open.map((r) => `${r.kind}:${r.label}:${r.depth}`)).toEqual([
      "folder:product:0",
      "folder:user:0",
      "folder:2:1",
      "key:cart:2",
      "key:2:1",
      "key:10:1",
      "key:counter:0",
    ]);
  });

  it("opens every folder while filtering", () => {
    const rows = browserRows(keys, true, "all");
    expect(rows.filter((r) => r.kind === "key")).toHaveLength(5);
    expect(rows.every((r) => r.kind !== "folder" || r.expanded)).toBe(true);
  });

  it("does not split binary key names", () => {
    const rows = browserRows([key("0xff3a00", "string", true)], true, new Set());
    expect(rows).toEqual([{ kind: "key", key: key("0xff3a00", "string", true), label: "0xff3a00", depth: 0 }]);
  });
});

describe("mergeKeys", () => {
  it("drops keys a later scan batch returned again", () => {
    expect(mergeKeys([[key("a"), key("b")], [key("b", "hash")]]).map((k) => `${k.key.name}:${k.kind}`)).toEqual([
      "a:string",
      "b:hash",
    ]);
  });
});

describe("formatting", () => {
  it("writes the time left in the two largest units", () => {
    expect(formatTtl(null)).toBe("不过期");
    expect(formatTtl(42_000)).toBe("42 秒");
    expect(formatTtl(1_752_000)).toBe("29 分 12 秒");
    expect(formatTtl(3_600_000)).toBe("1 小时");
    expect(formatTtl(93_780_000)).toBe("1 天 2 小时");
  });

  it("counts strings in bytes and collections in items", () => {
    expect(formatLength("string", 1024)).toBe("1,024 字节");
    expect(formatLength("hash", 4)).toBe("4 个字段");
    expect(formatLength("zset", 8)).toBe("8 个成员");
    expect(formatLength("list", null)).toBeNull();
  });

  it("parses the TTL field", () => {
    expect(parseTtl("")).toEqual({ ok: true, seconds: null });
    expect(parseTtl(" 60 ")).toEqual({ ok: true, seconds: 60 });
    expect(parseTtl("0")).toMatchObject({ ok: false });
    expect(parseTtl("1.5")).toMatchObject({ ok: false });
    expect(parseTtl(String(MAX_TTL_SECONDS))).toEqual({ ok: true, seconds: MAX_TTL_SECONDS });
    expect(parseTtl(String(MAX_TTL_SECONDS + 1))).toMatchObject({ ok: false });
  });
});
