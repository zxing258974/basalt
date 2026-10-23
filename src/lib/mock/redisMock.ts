// mock 后端的 Redis:一个商城应用的缓存,各种类型都有几个,方便在浏览器里看键浏览和命令行。
// 0 号库是应用缓存,1 号库是队列,5 号库留着一个旧键;其余是空库。
import type { RedisKeyspace } from "../redisTypes";

export type MockRedisValue =
  | { type: "string"; value: string }
  | { type: "hash"; value: [string, string][] }
  | { type: "list"; value: string[] }
  | { type: "set"; value: string[] }
  | { type: "zset"; value: [string, number][] }
  | { type: "stream"; value: { id: string; fields: [string, string][] }[] };

export interface MockRedisKey {
  value: MockRedisValue;
  /** 到期时刻(毫秒);不过期为 null。 */
  expiresAt: number | null;
}

/** 服务器上的库数,和 Redis 的默认值一样。 */
const DATABASES = 16;

const NAMES = ["黄勇", "李娜", "吴霞", "周杰", "刘洋", "Emily Chen", "张伟", "王芳"];
const CITIES = ["杭州", "上海", "北京", "深圳", "成都", "武汉"];
const PRODUCTS = ["机械键盘", "降噪耳机", "保温杯", "帆布包", "台灯", "鼠标垫", "充电宝", "雨伞"];

function seconds(n: number): number {
  return Date.now() + n * 1000;
}

function seedCache(): Map<string, MockRedisKey> {
  const keys = new Map<string, MockRedisKey>();
  const put = (key: string, value: MockRedisValue, ttl: number | null = null) =>
    keys.set(key, { value, expiresAt: ttl === null ? null : seconds(ttl) });

  put(
    "session:7f3a9c2e",
    {
      type: "hash",
      value: [
        ["user_id", "1001"],
        ["ip", "10.0.8.31"],
        ["ua", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15"],
        ["created_at", "2026-10-23T08:12:44Z"],
      ],
    },
    1800,
  );
  put(
    "session:b81d4e07",
    {
      type: "hash",
      value: [
        ["user_id", "1002"],
        ["ip", "10.0.8.40"],
        ["created_at", "2026-10-23T08:40:02Z"],
      ],
    },
    1200,
  );
  NAMES.forEach((name, i) =>
    put(`user:${1001 + i}`, {
      type: "hash",
      value: [
        ["name", name],
        ["email", `user${i + 1}@example.com`],
        ["city", CITIES[i % CITIES.length]!],
        ["vip", i % 4 === 3 ? "1" : "0"],
      ],
    }),
  );
  put("user:1001:cart", { type: "list", value: ["2003", "2017", "2017", "2040"] });
  for (let i = 0; i < 40; i += 1) {
    const id = 2001 + i;
    const product = {
      id,
      name: `${PRODUCTS[i % PRODUCTS.length]} ${Math.floor(i / PRODUCTS.length) + 1} 代`,
      price: Math.round((39 + ((i * 37) % 900) + 0.9) * 100) / 100,
      stock: (i * 13) % 120,
      tags: i % 3 === 0 ? ["新品", "包邮"] : ["包邮"],
      updatedAt: `2026-10-${String(1 + (i % 22)).padStart(2, "0")}T09:30:00Z`,
    };
    put(`product:${id}`, { type: "string", value: JSON.stringify(product) }, i % 5 === 0 ? 3600 : null);
  }
  put("leaderboard:weekly", {
    type: "zset",
    value: NAMES.map((name, i): [string, number] => [name, Math.round((3360.93 - i * 377.5) * 100) / 100]),
  });
  put("tags:popular", { type: "set", value: ["新品", "包邮", "满减", "会员价", "限时"] });
  put("orders:events", {
    type: "stream",
    value: ["paid", "shipped", "paid", "refunded", "paid"].map((status, i) => ({
      id: `1729650${String(i * 4321).padStart(6, "0")}-0`,
      fields: [
        ["order_id", String(900719925474001 + i)],
        ["status", status],
        ["amount", (814.18 - i * 97.3).toFixed(2)],
      ],
    })),
  });
  put("config:feature_flags", {
    type: "string",
    value:
      '{"newCheckout":true,"darkMode":false,"maxCartItems":50,"banner":{"text":"双十一预热","until":"2026-11-11"}}',
  });
  put("rate:ip:10.0.8.31", { type: "string", value: "37" }, 42);
  put("counter:visits", { type: "string", value: "1048576" });
  return keys;
}

function seedQueues(): Map<string, MockRedisKey> {
  const task = (kind: string, to: string, i: number) =>
    JSON.stringify({ kind, to, template: "order_shipped", orderId: 900719925474001 + i });
  return new Map<string, MockRedisKey>([
    [
      "queue:email",
      {
        value: { type: "list", value: [0, 1, 2].map((i) => task("email", `user${i + 1}@example.com`, i)) },
        expiresAt: null,
      },
    ],
    ["queue:sms", { value: { type: "list", value: [task("sms", "138****0001", 3)] }, expiresAt: null }],
    ["lock:daily-report", { value: { type: "string", value: "worker-2" }, expiresAt: seconds(90) }],
  ]);
}

export function createMockRedis() {
  const dbs = new Map<number, Map<string, MockRedisKey>>([
    [0, seedCache()],
    [1, seedQueues()],
    [5, new Map([["cache:legacy-home", { value: { type: "string", value: "<html>…</html>" }, expiresAt: null }]])],
  ]);

  /** 过期的键当作不存在,顺手删掉(Redis 也是访问时才清)。 */
  const live = (db: number): Map<string, MockRedisKey> => {
    const keys = dbs.get(db) ?? new Map<string, MockRedisKey>();
    dbs.set(db, keys);
    const now = Date.now();
    for (const [key, entry] of keys) if (entry.expiresAt !== null && entry.expiresAt <= now) keys.delete(key);
    return keys;
  };

  return {
    db: live,
    keyspace(defaultDb: number): RedisKeyspace {
      const databases = Array.from({ length: DATABASES }, (_, index) => {
        const keys = [...live(index).values()];
        return { index, keys: keys.length, expires: keys.filter((k) => k.expiresAt !== null).length };
      });
      return { count: DATABASES, databases, defaultDb };
    },
  };
}

export type MockRedis = ReturnType<typeof createMockRedis>;
