import { describe, expect, it } from "vitest";
import { carriesSecret, commandVerb, dangerReason, refusedReason, splitArgs } from "./redisArgs";

describe("splitArgs", () => {
  it("splits like redis-cli", () => {
    expect(splitArgs("  set  k   v ")).toEqual(["set", "k", "v"]);
    expect(splitArgs(`set "a b" 'c d'`)).toEqual(["set", "a b", "c d"]);
    expect(splitArgs(`set k "line\\none\\t\\"q\\""`)).toEqual(["set", "k", 'line\none\t"q"']);
    expect(splitArgs(`set k 'it\\'s \\n'`)).toEqual(["set", "k", "it's \\n"]);
    expect(splitArgs(`set k "\\xe4\\xbd\\xa0好"`)).toEqual(["set", "k", "你好"]);
    expect(splitArgs("hset 用户:1 名字 张三")).toEqual(["hset", "用户:1", "名字", "张三"]);
    expect(splitArgs(`set k ""`)).toEqual(["set", "k", ""]);
    expect(splitArgs("   ")).toEqual([]);
  });

  it("explains unbalanced quotes", () => {
    expect(splitArgs(`set k "open`)).toEqual({ error: "引号没有配对" });
    expect(splitArgs(`set k "a"b`)).toEqual({ error: "引号收尾后面要有空格" });
  });
});

describe("command classes", () => {
  const verb = (line: string) => commandVerb(splitArgs(line) as string[]);

  it("names commands with their subcommand where it matters", () => {
    expect(verb("config set maxmemory 1gb")).toBe("CONFIG SET");
    expect(verb("get config")).toBe("GET");
    expect(verb("client")).toBe("CLIENT");
  });

  it("asks before dangerous commands and refuses subscriptions", () => {
    expect(dangerReason(verb("flushall async"))).toMatch(/所有库/);
    expect(dangerReason(verb("keys *"))).toMatch(/卡住服务器/);
    expect(dangerReason(verb("debug sleep 1"))).toMatch(/卡住或崩溃/);
    expect(dangerReason(verb("config get maxmemory"))).toBeNull();
    expect(refusedReason(verb("psubscribe news.*"))).toMatch(/订阅/);
    expect(refusedReason(verb("sunsubscribe"))).toMatch(/订阅/);
    expect(refusedReason(verb("monitor"))).toMatch(/MONITOR/);
    expect(refusedReason(verb("client reply off"))).toMatch(/收不到回复/);
    expect(refusedReason(verb("publish news hi"))).toBeNull();
  });

  it("spots commands that carry a password", () => {
    const secret = (line: string) => carriesSecret(splitArgs(line) as string[]);
    expect(secret("auth hunter2")).toBe(true);
    expect(secret("AUTH app hunter2")).toBe(true);
    expect(secret("hello 3 auth app hunter2")).toBe(true);
    expect(secret("acl setuser app on >hunter2")).toBe(true);
    expect(secret("config set requirepass hunter2")).toBe(true);
    expect(secret("config set maxmemory 1gb")).toBe(false);
    expect(secret("get auth")).toBe(false);
  });
});
