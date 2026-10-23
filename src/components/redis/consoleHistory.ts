// 命令行的历史:每个连接一份,记在本机存储里(只是方便,读写失败就当没有)。带密码的命令不记。
import { carriesSecret, splitArgs } from "../../lib/redisArgs";

export const HISTORY_LIMIT = 500;

const storageKey = (connectionId: string) => `basalt.redis.history.${connectionId}`;

export function loadHistory(connectionId: string): string[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(storageKey(connectionId)) ?? "[]");
    return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string").slice(-HISTORY_LIMIT) : [];
  } catch {
    return [];
  }
}

/** 加上一条:和上一条一样的不重复记,带密码的不记,最多留 `HISTORY_LIMIT` 条。 */
export function withCommand(history: readonly string[], line: string): readonly string[] {
  const args = splitArgs(line);
  if ("error" in args || args.length === 0 || carriesSecret(args)) return history;
  if (history.at(-1) === line) return history;
  return [...history, line].slice(-HISTORY_LIMIT);
}

/** 记下一条,返回记完的历史。几个命令行标签页共用一份:先读存储里最新的再加。 */
export function rememberCommand(connectionId: string, line: string): readonly string[] {
  const next = withCommand(loadHistory(connectionId), line);
  try {
    localStorage.setItem(storageKey(connectionId), JSON.stringify(next));
  } catch (error) {
    console.warn("无法保存命令行历史", error);
  }
  return next;
}
