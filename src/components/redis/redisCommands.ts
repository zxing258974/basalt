// 命令行补全和参数提示:常用命令的写法和一句话说明。服务器上别的命令(模块的、新版本的)只补全名字。

export interface CommandHint {
  args: string;
  summary: string;
}

const HINTS: Record<string, [string, string]> = {
  // 通用
  DEL: ["key [key ...]", "删除键"],
  UNLINK: ["key [key ...]", "删除键，大键在后台释放"],
  EXISTS: ["key [key ...]", "有几个键存在"],
  TYPE: ["key", "键的类型"],
  TTL: ["key", "剩余秒数；-1 不过期，-2 不存在"],
  PTTL: ["key", "剩余毫秒数"],
  EXPIRE: ["key seconds [NX|XX|GT|LT]", "设过期秒数"],
  PEXPIRE: ["key milliseconds [NX|XX|GT|LT]", "设过期毫秒数"],
  PERSIST: ["key", "去掉过期时间"],
  RENAME: ["key newkey", "改名（会覆盖已有的）"],
  RENAMENX: ["key newkey", "新名字没人用才改名"],
  SCAN: ["cursor [MATCH pattern] [COUNT count] [TYPE type]", "分批遍历键"],
  KEYS: ["pattern", "一次列出匹配的键（大库会卡住服务器）"],
  DUMP: ["key", "把值序列化"],
  OBJECT: ["ENCODING|FREQ|IDLETIME|REFCOUNT key", "值的内部信息"],
  MEMORY: ["USAGE key [SAMPLES count]", "键占多少内存"],
  RANDOMKEY: ["", "随机一个键"],
  DBSIZE: ["", "当前库的键数"],
  SELECT: ["index", "换到另一个库"],
  MOVE: ["key db", "把键挪到另一个库"],
  COPY: ["source destination [DB db] [REPLACE]", "复制键"],
  // 字符串
  GET: ["key", "取字符串"],
  SET: ["key value [NX|XX] [GET] [EX seconds|PX milliseconds|KEEPTTL]", "写字符串"],
  SETNX: ["key value", "不存在才写"],
  SETEX: ["key seconds value", "写并设过期秒数"],
  GETDEL: ["key", "取出并删除"],
  GETEX: ["key [EX seconds|PX milliseconds|PERSIST]", "取值并改过期时间"],
  MGET: ["key [key ...]", "一次取多个"],
  MSET: ["key value [key value ...]", "一次写多个"],
  APPEND: ["key value", "追加到末尾"],
  STRLEN: ["key", "字节数"],
  GETRANGE: ["key start end", "取一段（按字节）"],
  SETRANGE: ["key offset value", "改一段（按字节）"],
  INCR: ["key", "加一"],
  INCRBY: ["key increment", "加上整数"],
  INCRBYFLOAT: ["key increment", "加上小数"],
  DECR: ["key", "减一"],
  DECRBY: ["key decrement", "减去整数"],
  // hash
  HGET: ["key field", "取字段"],
  HSET: ["key field value [field value ...]", "写字段"],
  HSETNX: ["key field value", "字段不存在才写"],
  HMGET: ["key field [field ...]", "取多个字段"],
  HGETALL: ["key", "全部字段和值"],
  HDEL: ["key field [field ...]", "删字段"],
  HEXISTS: ["key field", "字段在不在"],
  HKEYS: ["key", "全部字段名"],
  HVALS: ["key", "全部值"],
  HLEN: ["key", "字段数"],
  HINCRBY: ["key field increment", "字段加上整数"],
  HSCAN: ["key cursor [MATCH pattern] [COUNT count]", "分批遍历字段"],
  // list
  LPUSH: ["key element [element ...]", "从左边插入"],
  RPUSH: ["key element [element ...]", "从右边插入"],
  LPOP: ["key [count]", "从左边弹出"],
  RPOP: ["key [count]", "从右边弹出"],
  LRANGE: ["key start stop", "取一段（-1 是最后一个）"],
  LINDEX: ["key index", "取第几个"],
  LSET: ["key index element", "改第几个"],
  LLEN: ["key", "元素个数"],
  LREM: ["key count element", "删掉等于它的元素"],
  LTRIM: ["key start stop", "只留一段"],
  BLPOP: ["key [key ...] timeout", "等着从左边弹出（0 一直等）"],
  BRPOP: ["key [key ...] timeout", "等着从右边弹出（0 一直等）"],
  // set
  SADD: ["key member [member ...]", "加成员"],
  SREM: ["key member [member ...]", "删成员"],
  SMEMBERS: ["key", "全部成员"],
  SISMEMBER: ["key member", "是不是成员"],
  SCARD: ["key", "成员数"],
  SINTER: ["key [key ...]", "交集"],
  SUNION: ["key [key ...]", "并集"],
  SDIFF: ["key [key ...]", "差集"],
  SSCAN: ["key cursor [MATCH pattern] [COUNT count]", "分批遍历成员"],
  // zset
  ZADD: ["key [NX|XX] [GT|LT] [CH] [INCR] score member [score member ...]", "加成员和分数"],
  ZREM: ["key member [member ...]", "删成员"],
  ZSCORE: ["key member", "成员的分数"],
  ZINCRBY: ["key increment member", "分数加上一个数"],
  ZCARD: ["key", "成员数"],
  ZRANGE: ["key start stop [BYSCORE|BYLEX] [REV] [LIMIT offset count] [WITHSCORES]", "按名次或分数取一段"],
  ZRANK: ["key member [WITHSCORE]", "从小到大排第几"],
  ZREVRANK: ["key member [WITHSCORE]", "从大到小排第几"],
  ZCOUNT: ["key min max", "分数在范围里的个数"],
  ZSCAN: ["key cursor [MATCH pattern] [COUNT count]", "分批遍历成员"],
  // stream
  XADD: ["key [MAXLEN [~] count] *|id field value [field value ...]", "追加消息"],
  XRANGE: ["key start end [COUNT count]", "按 ID 取一段（- + 是全部）"],
  XREVRANGE: ["key end start [COUNT count]", "倒着取一段"],
  XLEN: ["key", "消息数"],
  XDEL: ["key id [id ...]", "删消息"],
  XINFO: ["STREAM|GROUPS|CONSUMERS key", "stream 的信息"],
  // 事务、脚本、发布
  MULTI: ["", "开始事务"],
  EXEC: ["", "执行事务里排队的命令"],
  DISCARD: ["", "放弃事务"],
  WATCH: ["key [key ...]", "盯住键，被改了 EXEC 就不执行"],
  EVAL: ["script numkeys [key ...] [arg ...]", "执行 Lua 脚本"],
  PUBLISH: ["channel message", "发布消息"],
  // 服务器
  PING: ["[message]", "测连通"],
  ECHO: ["message", "原样返回"],
  INFO: ["[section ...]", "服务器信息和统计"],
  CONFIG: ["GET parameter | SET parameter value", "读写运行配置"],
  CLIENT: ["LIST | INFO | KILL ID id | SETNAME name", "客户端连接"],
  SLOWLOG: ["GET [count] | LEN | RESET", "慢查询日志"],
  TIME: ["", "服务器时间"],
  FLUSHDB: ["[ASYNC|SYNC]", "清空当前库"],
  FLUSHALL: ["[ASYNC|SYNC]", "清空所有库"],
  AUTH: ["[username] password", "换个账号认证"],
};

export function commandHint(name: string): CommandHint | null {
  const hint = HINTS[name.toUpperCase()];
  return hint ? { args: hint[0], summary: hint[1] } : null;
}

/** 以 `prefix` 开头的命令名:常用的(有提示的)排前面,最多 `limit` 个。 */
export function completeCommand(prefix: string, serverNames: readonly string[], limit = 8): string[] {
  const upper = prefix.toUpperCase();
  if (!upper) return [];
  const known = Object.keys(HINTS).filter((n) => n.startsWith(upper));
  const others = serverNames.filter((n) => n.startsWith(upper) && !(n in HINTS) && !n.includes("|"));
  return [...known.sort(), ...[...others].sort()].slice(0, limit);
}
