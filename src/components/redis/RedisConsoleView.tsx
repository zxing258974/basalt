import { useQuery } from "@tanstack/react-query";
import { Eraser, Square } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { catalogKeys, useConnections } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { getBackend } from "../../lib/api";
import { errorMessage, isCommandError, type ConnectionView } from "../../lib/types";
import type { RedisConsoleTab } from "../../stores/tabs";
import { DriverBadge } from "../connection/DriverBadge";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { EmptyState } from "../ui/EmptyState";
import { ConsoleEntries, type ConsoleEntry, type EntryResult } from "./ConsoleEntries";
import { ConsoleInput } from "./ConsoleInput";
import { loadHistory, rememberCommand } from "./consoleHistory";
import { commandLines, displayLine, promptText, verbOf } from "./consoleModel";

/** 滚动区最多留这么多条,更早的丢掉。 */
const MAX_ENTRIES = 1000;

/** Redis 命令行标签页:连接关了就请用户重新打开。 */
export function RedisConsoleView({ tab, active }: { tab: RedisConsoleTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const { openWithFeedback } = useConnectionActions();
  if (!connection) {
    return (
      <section className="redis-console">
        <EmptyState title="这个连接已经不存在" />
      </section>
    );
  }
  if (!connection.isOpen) {
    return (
      <section className="redis-console">
        <EmptyState
          title="连接已关闭"
          detail="重新打开之后，这个命令行回到它打开时的库，事务需要重新开始。"
          action={
            <Button variant="primary" onClick={() => void openWithFeedback(connection)}>
              重新打开
            </Button>
          }
        />
      </section>
    );
  }
  return <Console connection={connection} tab={tab} active={active} />;
}

interface Confirm {
  verb: string;
  reason: string;
  db: number;
  decide(run: boolean): void;
}

function Console({ connection, tab, active }: { connection: ConnectionView; tab: RedisConsoleTab; active: boolean }) {
  const [entries, setEntries] = useState<ConsoleEntry[]>([]);
  const [db, setDb] = useState(tab.db);
  const [transaction, setTransaction] = useState(false);
  const [running, setRunning] = useState(false);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [history, setHistory] = useState<readonly string[]>(() => loadHistory(connection.id));
  // 一批命令依次执行时,下一条要用上一条执行完的库号:放在 ref 里,不等重新渲染。
  const where = useRef({ db: tab.db, transaction: false });
  const stopped = useRef(false);
  // 标签页关掉之后,正在依次执行的那批命令不再往下跑。
  const alive = useRef(true);
  const nextId = useRef(1);
  const logRef = useRef<HTMLDivElement>(null);
  const [focusRequest, setFocusRequest] = useState(0);
  const names = useQuery({
    queryKey: [...catalogKeys.connection(connection.id), "redis", "commands"],
    queryFn: async () => (await getBackend()).redisCommandNames(connection.id),
    staleTime: Infinity,
    retry: false,
  });

  // 标签页关掉(或连接关掉、应用退出)时断开这个命令行的连接。
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      void getBackend()
        .then((backend) => backend.redisConsoleClose(connection.id, tab.id))
        .catch((error: unknown) => console.warn("关闭命令行的连接失败", error));
    };
  }, [connection.id, tab.id]);

  useLayoutEffect(() => {
    const log = logRef.current;
    if (log) log.scrollTop = log.scrollHeight;
  }, [entries]);

  const host =
    connection.endpoint.kind === "network" ? `${connection.endpoint.host}:${connection.endpoint.port}` : "redis";

  const settle = (id: number, result: EntryResult) =>
    setEntries((list) => list.map((e) => (e.id === id ? { ...e, result } : e)));

  const ask = (verb: string, reason: string) =>
    new Promise<boolean>((resolve) =>
      setConfirm({
        verb,
        reason,
        db: where.current.db,
        decide: (run) => {
          setConfirm(null);
          setFocusRequest((n) => n + 1);
          resolve(run);
        },
      }),
    );

  /** 执行一条;没执行成(失败、没确认、被停下)返回 false,后面的不再执行。 */
  const runOne = async (line: string): Promise<boolean> => {
    const id = nextId.current++;
    const verb = verbOf(line);
    const prompt = promptText(host, where.current.db, where.current.transaction);
    setEntries((list) =>
      [...list, { id, prompt, shown: displayLine(line), verb, result: { kind: "pending" as const } }].slice(
        -MAX_ENTRIES,
      ),
    );
    setHistory(rememberCommand(connection.id, line));
    try {
      const backend = await getBackend();
      const send = (confirmed: boolean) =>
        backend.redisConsoleRun(connection.id, tab.id, where.current.db, line, confirmed);
      let outcome = await send(false);
      if (outcome.outcome === "confirm") {
        settle(id, { kind: "asking" });
        if (!(await ask(verb, outcome.reason))) {
          settle(id, { kind: "declined" });
          return false;
        }
        settle(id, { kind: "pending" });
        outcome = await send(true);
      }
      if (outcome.outcome !== "ran") throw new Error("确认之后服务器仍要求确认");
      where.current = { db: outcome.db, transaction: outcome.transaction };
      setDb(outcome.db);
      setTransaction(outcome.transaction);
      settle(id, { kind: "reply", reply: outcome.reply, elapsedMs: outcome.elapsedMs });
      return true;
    } catch (failure) {
      // 连接断了(或被停下)事务就没了;下一条命令重新连到界面上的库。
      const dropped = stopped.current || (isCommandError(failure) && failure.code === "network");
      if (dropped) {
        where.current = { ...where.current, transaction: false };
        setTransaction(false);
      }
      settle(id, {
        kind: "failure",
        message: stopped.current
          ? "已停下：这个命令行的连接断开了，下一条命令会重新连接（事务、WATCH 和用 AUTH 换的账号都已经失效）。"
          : errorMessage(failure),
      });
      return false;
    }
  };

  const run = async (text: string) => {
    if (running) return;
    stopped.current = false;
    setRunning(true);
    try {
      for (const line of commandLines(text)) {
        // 停下了、标签页关了:剩下的不再执行。
        if (stopped.current || !alive.current) break;
        if (!(await runOne(line))) break;
      }
    } finally {
      setRunning(false);
    }
  };

  const cancel = () => {
    if (!running) return;
    // 还在等确认:就是不执行。
    if (confirm) {
      confirm.decide(false);
      return;
    }
    stopped.current = true;
    void getBackend()
      .then((backend) => backend.redisConsoleCancel(connection.id, tab.id))
      .catch((error: unknown) => console.warn("停下命令失败", error));
  };

  return (
    <section className="redis-console" aria-label={`${connection.name} ${tab.title}`}>
      <header className="data-header console-header">
        <div className="data-heading">
          <DriverBadge driver={connection.driver} />
          <span className="data-crumb">{connection.name}</span>
          <span className="data-crumb-sep">/</span>
          <h1 className="data-title">{tab.title}</h1>
          <span className="console-where">
            <span className="mono">db{db}</span>
            {transaction && <span className="console-tx">事务中</span>}
          </span>
          <span className="console-actions">
            {running && (
              <Button
                size="sm"
                variant="secondary"
                icon={<Square />}
                title="Esc"
                onClick={() => {
                  cancel();
                  // 按钮在命令停下后就没了,焦点不能跟着它丢掉。
                  setFocusRequest((n) => n + 1);
                }}
              >
                停止
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              icon={<Eraser />}
              disabled={entries.length === 0}
              onClick={() => setEntries([])}
            >
              清屏
            </Button>
          </span>
        </div>
      </header>

      <div
        ref={logRef}
        className="console-log"
        role="log"
        aria-label="命令行输出"
        onMouseUp={() => {
          // 像终端一样:点一下空白处回到输入框;选中文字(要复制)时不抢。
          if (window.getSelection()?.isCollapsed ?? true) setFocusRequest((n) => n + 1);
        }}
      >
        <ConsoleEntries entries={entries} />
      </div>

      <ConsoleInput
        focusRequest={focusRequest}
        prompt={promptText(host, db, transaction)}
        running={running}
        active={active}
        serverNames={names.data ?? []}
        history={history}
        onRun={(text) => void run(text)}
        onCancel={cancel}
        onClear={() => setEntries([])}
      />

      {/* 在后台标签页里跑到危险命令:等切回来再问,不在看不见的地方弹框。 */}
      {confirm && active && (
        <Dialog
          open
          title={`执行 ${confirm.verb}？`}
          subtitle={confirm.reason}
          width={460}
          onClose={() => confirm.decide(false)}
          footer={
            <>
              <Button variant="secondary" autoFocus onClick={() => confirm.decide(false)}>
                取消
              </Button>
              <Button variant="danger" onClick={() => confirm.decide(true)}>
                执行
              </Button>
            </>
          }
        >
          <p className="console-confirm-where">
            在 <span className="mono">{promptText(host, confirm.db, false).slice(0, -1)}</span> 上执行。
          </p>
        </Dialog>
      )}
    </section>
  );
}
