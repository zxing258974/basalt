import { describe, expect, it } from "vitest";
import type { Reply } from "../../lib/redisTypes";
import { withCommand } from "./consoleHistory";
import { commandLines, displayLine, formatReply, promptText, quoteText, type OutputLine } from "./consoleModel";
import { commandHint, completeCommand } from "./redisCommands";

const bulk = (text: string): Reply => ({
  kind: "bulk",
  text,
  binary: false,
  bytes: new TextEncoder().encode(text).length,
  truncated: false,
});
const array = (items: Reply[]): Reply => ({ kind: "array", items, total: items.length });
/** 每行拼成一个字符串,好比对。 */
const text = (lines: OutputLine[]) => lines.map((l) => l.prefix + l.segments.map((s) => s.text).join(""));

describe("formatReply", () => {
  it("writes simple replies the way redis-cli does", () => {
    expect(text(formatReply({ kind: "nil" }))).toEqual(["(nil)"]);
    expect(text(formatReply({ kind: "int", value: "9007199254740993" }))).toEqual(["(integer) 9007199254740993"]);
    expect(text(formatReply({ kind: "status", text: "OK" }))).toEqual(["OK"]);
    expect(text(formatReply({ kind: "error", text: "ERR unknown command" }))).toEqual(["(error) ERR unknown command"]);
    expect(text(formatReply(bulk('说 "hi"\n')))).toEqual(['"说 \\"hi\\"\\n"']);
    expect(text(formatReply({ kind: "bulk", text: "\\x00\\xff", binary: true, bytes: 2, truncated: false }))).toEqual([
      '"\\x00\\xff"',
    ]);
    expect(text(formatReply(array([])))).toEqual(["(empty array)"]);
    expect(formatReply({ kind: "error", text: "x" })[0]!.segments[0]!.tone).toBe("error");
  });

  it("numbers arrays and indents nested ones under their number", () => {
    const reply = array([
      bulk("a"),
      array([bulk("x"), { kind: "int", value: "2" }]),
      ...Array.from({ length: 8 }, (_, i) => bulk(String(i))),
    ]);
    const lines = text(formatReply(reply));
    expect(lines.slice(0, 3)).toEqual([' 1) "a"', ' 2) 1) "x"', "    2) (integer) 2"]);
    expect(lines.at(-1)).toBe('10) "7"');
  });

  it("says when only part of a reply came back", () => {
    const partial: Reply = { kind: "array", items: [bulk("a")], total: 3 };
    expect(text(formatReply(partial)).at(-1)).toBe("…还有 2 项没显示");
    const cut: Reply = { kind: "bulk", text: "abc", binary: false, bytes: 2_000_000, truncated: true };
    expect(text(formatReply(cut)).at(-1)).toMatch(/只显示了前 3 字节/);
    const binary: Reply = { kind: "bulk", text: "\\xff", binary: true, bytes: 2_000_000, truncated: true };
    expect(text(formatReply(binary)).at(-1)).toMatch(/只显示了前面一部分，一共 2,000,000 字节/);
  });

  it("prints INFO and CLIENT LIST as plain lines", () => {
    const info = bulk("# Server\r\nredis_version:7.4.1\r\n");
    expect(text(formatReply(info, "INFO"))).toEqual(["# Server", "redis_version:7.4.1"]);
    expect(text(formatReply(info, "GET"))).toEqual(['"# Server\\r\\nredis_version:7.4.1\\r\\n"']);
  });

  it("writes maps with arrows", () => {
    const map: Reply = { kind: "map", entries: [[bulk("k"), bulk("v")]], total: 1 };
    expect(text(formatReply(map))).toEqual(['1# "k" => "v"']);
  });
});

describe("console helpers", () => {
  it("shows the database and transaction in the prompt", () => {
    expect(promptText("127.0.0.1:6379", 0, false)).toBe("127.0.0.1:6379>");
    expect(promptText("127.0.0.1:6379", 3, true)).toBe("127.0.0.1:6379[3](TX)>");
  });

  it("hides passwords when echoing commands", () => {
    expect(displayLine("auth app hunter2")).toBe("AUTH ••••••");
    expect(displayLine("config set requirepass x")).toBe("CONFIG SET ••••••");
    expect(displayLine("get k")).toBe("get k");
    expect(displayLine('auth "p@ss')).toBe("AUTH ••••••");
    expect(displayLine("config set primaryauth s3cret")).toBe("CONFIG SET ••••••");
    expect(displayLine('get "open')).toBe('get "open');
  });

  it("runs pasted lines one by one", () => {
    expect(commandLines("set a 1\n\n  get a  \r\n")).toEqual(["set a 1", "get a"]);
  });

  it("quotes text like redis-cli", () => {
    expect(quoteText("a\tb\x01")).toBe('"a\\tb\\x01"');
  });

  it("keeps history without repeats or passwords", () => {
    let history: readonly string[] = [];
    history = withCommand(history, "get a");
    history = withCommand(history, "get a");
    history = withCommand(history, "auth hunter2");
    history = withCommand(history, 'set k "open');
    expect(history).toEqual(["get a"]);
  });

  it("completes command names, common ones first", () => {
    expect(completeCommand("hg", ["HGET", "HGETALL", "HGETDEL", "HELLO"])).toEqual(["HGET", "HGETALL", "HGETDEL"]);
    expect(completeCommand("", ["GET"])).toEqual([]);
    expect(completeCommand("client", ["CLIENT", "CLIENT|LIST"])).toEqual(["CLIENT"]);
    expect(commandHint("set")?.args).toMatch(/^key value/);
    expect(commandHint("NOPE")).toBeNull();
  });
});
