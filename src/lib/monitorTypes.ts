// 服务器监控:和 basalt-core 的 monitor 模块一一对应(serde camelCase)。
// 计数器只增不减,速率由前端拿前后两份快照相减、除以间隔算出来。

/** 一个会话(MySQL 的线程、PostgreSQL 的后端进程)。 */
export interface ServerProcess {
  id: number;
  user: string | null;
  /** 客户端地址。 */
  host: string | null;
  database: string | null;
  /** MySQL 的 Command(Query、Sleep……),PostgreSQL 的 state(active、idle in transaction……)。 */
  state: string | null;
  /** MySQL 的 State,PostgreSQL 在等什么(wait_event_type:wait_event)。 */
  detail: string | null;
  /** 处在当前状态多少秒了。 */
  seconds: number | null;
  /** 最多 4096 个字。 */
  query: string | null;
  activity: ProcessActivity;
  /** PostgreSQL 的 backend_start:终止前确认还是同一个会话(进程号会给新会话用)。 */
  sessionStart: string | null;
  /** Basalt 取监控数据用的那条连接。 */
  isSelf: boolean;
}

/**
 * 正在执行语句;开着事务闲着(PostgreSQL 的 idle in transaction,可能占着锁);空闲;
 * 服务器自己的线程(事件调度器、复制);没权限看。
 */
export type ProcessActivity = "running" | "inTransaction" | "idle" | "background" | "unknown";

/** 读不到的(没权限、这种数据库没有)为 null。 */
export interface ServerCounters {
  uptimeSeconds: number | null;
  connections: number;
  maxConnections: number | null;
  running: number;
  queries: number | null;
  commits: number | null;
  rollbacks: number | null;
  bytesReceived: number | null;
  bytesSent: number | null;
  /** 累计读数据页次数(含命中缓存的),和其中要读盘的次数。 */
  cacheReads: number | null;
  cacheMisses: number | null;
  slowQueries: number | null;
}

export interface ServerSnapshot {
  counters: ServerCounters;
  processes: ServerProcess[];
  /** Unix 毫秒。 */
  takenAtMs: number;
  /** 账号权限不够、只看得到一部分时的说明。 */
  limited: string | null;
}

/** 只停掉正在执行的语句,还是连同连接一起断开。 */
export type KillMode = "query" | "connection";

/** 要终止的会话,连同确认时看到的样子:后端对不上(换了语句、会话已经结束)就不动手。 */
export interface KillTarget {
  id: number;
  mode: KillMode;
  sessionStart: string | null;
  query: string | null;
}
