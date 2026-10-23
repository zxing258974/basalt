import { useMemo, useState } from "react";
import type { Reply } from "../../lib/redisTypes";
import { formatReply, type OutputLine } from "./consoleModel";

/** 一条输出先显示这么多行,再多的点了才显示(几万项的数组一次画出来太慢)。 */
const FIRST_LINES = 1000;

export type EntryResult =
  | { kind: "pending" }
  /** 危险命令,等用户在确认框里决定。 */
  | { kind: "asking" }
  | { kind: "reply"; reply: Reply; elapsedMs: number }
  /** 没执行成(拆不了、不支持、断开了……)。 */
  | { kind: "failure"; message: string }
  /** 危险命令,用户没确认。 */
  | { kind: "declined" };

export interface ConsoleEntry {
  id: number;
  prompt: string;
  /** 回显的命令(密码已遮住)。 */
  shown: string;
  verb: string;
  result: EntryResult;
}

/** 命令行上面的滚动区:一条条命令和它的回复。 */
export function ConsoleEntries({ entries }: { entries: readonly ConsoleEntry[] }) {
  if (entries.length === 0) {
    return (
      <div className="console-intro">
        <p>输入 Redis 命令，回车执行，回复照 redis-cli 的样子显示。</p>
        <p>
          这个标签页有自己的连接：<code>SELECT</code> 换库、<code>MULTI</code> 开事务只影响这里。
          <code>FLUSHDB</code>、<code>KEYS</code> 这类危险的命令执行前会先问一句；订阅和 <code>MONITOR</code> 暂不支持。
        </p>
      </div>
    );
  }
  return (
    <>
      {entries.map((entry) => (
        <section key={entry.id} className="console-entry" aria-label={entry.shown}>
          <div className="console-command mono">
            <span className="console-prompt">{entry.prompt}</span>
            <span className="console-line selectable">{entry.shown}</span>
            {entry.result.kind === "reply" && (
              <span className="console-elapsed" title="服务器回复用了多久（含网络）">
                {formatElapsed(entry.result.elapsedMs)}
              </span>
            )}
          </div>
          <EntryOutput entry={entry} />
        </section>
      ))}
    </>
  );
}

function EntryOutput({ entry }: { entry: ConsoleEntry }) {
  const { result } = entry;
  switch (result.kind) {
    case "pending":
      return (
        <p className="console-pending">
          <span className="spinner" aria-hidden />
          正在等回复…
        </p>
      );
    case "asking":
      return <p className="console-pending">等你确认…</p>;
    case "reply":
      return <ReplyOutput reply={result.reply} verb={entry.verb} />;
    case "failure":
      return (
        <p className="console-failure" role="alert">
          {result.message}
        </p>
      );
    case "declined":
      return <p className="console-declined">没有执行。</p>;
  }
}

function ReplyOutput({ reply, verb }: { reply: Reply; verb: string }) {
  const lines = useMemo(() => formatReply(reply, verb), [reply, verb]);
  const [all, setAll] = useState(false);
  const shown = all ? lines : lines.slice(0, FIRST_LINES);
  return (
    <div className="console-output mono selectable">
      {shown.map((line, i) => (
        <Line key={i} line={line} />
      ))}
      {shown.length < lines.length && (
        <button type="button" className="console-more" onClick={() => setAll(true)}>
          显示其余 {(lines.length - shown.length).toLocaleString()} 行
        </button>
      )}
    </div>
  );
}

function Line({ line }: { line: OutputLine }) {
  return (
    <div className="console-output-line">
      {line.prefix && <span className="console-index">{line.prefix}</span>}
      <span className="console-value">
        {line.segments.map((segment, i) => (
          <span key={i} className={`tone-${segment.tone}`}>
            {segment.text}
          </span>
        ))}
      </span>
    </div>
  );
}

function formatElapsed(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}
