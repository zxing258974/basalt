// Redis 的数据结构,与 crates/basalt-core/src/redis_db 的序列化结构一一对应。

/** 一个库:键数和设了过期时间的键数。 */
export interface RedisDatabase {
  index: number;
  keys: number;
  expires: number;
}

export interface RedisKeyspace {
  /** 服务器上一共有几个库。 */
  count: number;
  /** 对象树里列出的库:不多于 16 个时全部列出,更多时只列有键的、0 号库和默认库。 */
  databases: RedisDatabase[];
  /** 连接配置里的库号。 */
  defaultDb: number;
}
