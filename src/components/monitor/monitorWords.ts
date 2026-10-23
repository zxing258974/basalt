// 会话列表的用词:SQL 数据库停下的是「语句」,Redis 一次只执行一条命令,能停下的是阻塞着在等的客户端。
import type { DriverKind } from "../../lib/types";

export interface MonitorWords {
  /** 列表头和计数里「在执行」的说法。 */
  running: string;
  databaseColumn: string;
  queryColumn: string;
  search: string;
  /** 服务器自己的连接(不让断开)是什么。 */
  background: string;
  stop: {
    label: string;
    /** 没有能停下的东西时按钮上的提示。 */
    nothing: string;
    aria(id: number): string;
    title(id: number): string;
    confirm: string;
    effect: string;
    done(id: number): string;
  };
  disconnectEffect: string;
}

const SQL: MonitorWords = {
  running: "在执行",
  databaseColumn: "数据库",
  queryColumn: "语句",
  search: "按用户、地址、语句筛选",
  background: "服务器自己的线程（事件调度器、复制）",
  stop: {
    label: "终止查询",
    nothing: "没有在执行的语句",
    aria: (id) => `终止会话 ${id} 正在执行的语句`,
    title: (id) => `终止会话 ${id} 的语句？`,
    confirm: "终止语句",
    effect: "只停掉正在执行的这条语句，连接还留着；在事务里的话，这条语句的改动会撤销。",
    done: (id) => `已终止会话 ${id} 的语句。`,
  },
  disconnectEffect: "连接会被断开，没提交的事务会回滚。用这条连接的程序得自己重连。",
};

const REDIS: MonitorWords = {
  running: "阻塞",
  databaseColumn: "库",
  queryColumn: "最近的命令",
  search: "按用户、地址、客户端名、命令筛选",
  background: "主从复制、MONITOR 用的连接",
  stop: {
    label: "解除阻塞",
    nothing: "没有在阻塞等待",
    aria: (id) => `解除会话 ${id} 的阻塞`,
    title: (id) => `解除会话 ${id} 的阻塞？`,
    confirm: "解除阻塞",
    effect: "它正在等的命令（BLPOP、XREAD BLOCK 这类）马上返回空结果，连接还留着。",
    done: (id) => `已解除会话 ${id} 的阻塞。`,
  },
  disconnectEffect: "连接会被断开，它的事务、WATCH 和订阅都会没了。用这条连接的程序得自己重连。",
};

export function monitorWords(driver: DriverKind | undefined): MonitorWords {
  return driver === "redis" ? REDIS : SQL;
}
