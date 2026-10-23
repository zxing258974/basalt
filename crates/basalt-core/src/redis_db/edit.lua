-- Basalt 提交一批修改:先全部核对,对得上才全部写入,对不上一项都不写。
-- KEYS[1] 是键;ARGV 每三项是一步:操作、参数一、参数二。
-- 核对失败返回错误「BASALT 第几步 原因」,由 Rust 那边换成人话。
local key = KEYS[1]
local tomb = '\0basalt:deleted'
local steps = {}
for i = 1, #ARGV, 3 do
  steps[#steps + 1] = { ARGV[i], ARGV[i + 1], ARGV[i + 2] }
end

local function fail(index, reason)
  return redis.error_reply('BASALT ' .. index .. ' ' .. reason)
end

for index, step in ipairs(steps) do
  local op, a, b = step[1], step[2], step[3]
  if op == 'type' then
    if redis.call('TYPE', key)['ok'] ~= a then return fail(index, 'type') end
  elseif op == 'absent' then
    if redis.call('EXISTS', key) == 1 then return fail(index, 'exists') end
  elseif op == 'eq_get' then
    if redis.call('GET', key) ~= a then return fail(index, 'changed') end
  elseif op == 'eq_hget' then
    if redis.call('HGET', key, a) ~= b then return fail(index, 'changed') end
  elseif op == 'no_hfield' then
    if redis.call('HEXISTS', key, a) == 1 then return fail(index, 'exists') end
  elseif op == 'eq_lindex' then
    if redis.call('LINDEX', key, tonumber(a)) ~= b then return fail(index, 'changed') end
  elseif op == 'is_member' then
    if redis.call('SISMEMBER', key, a) == 0 then return fail(index, 'changed') end
  elseif op == 'no_member' then
    if redis.call('SISMEMBER', key, a) == 1 then return fail(index, 'exists') end
  elseif op == 'eq_zscore' then
    if redis.call('ZSCORE', key, a) ~= b then return fail(index, 'changed') end
  elseif op == 'no_zmember' then
    if redis.call('ZSCORE', key, a) then return fail(index, 'exists') end
  end
end

-- 改值不改剩余时间:SET 会清掉过期时间(老版本没有 SET … KEEPTTL),集合中途删空时键会被删掉、
-- 再加进去就是个不过期的新键。写完如果没有过期时间了,按原来剩下的补回去。
local ttl = redis.call('PTTL', key)
local marked = false
local written = 0
for _, step in ipairs(steps) do
  local op, a, b = step[1], step[2], step[3]
  if op == 'set' then redis.call('SET', key, a)
  elseif op == 'hset' then redis.call('HSET', key, a, b)
  elseif op == 'hdel' then redis.call('HDEL', key, a)
  elseif op == 'lset' then redis.call('LSET', key, tonumber(a), b)
  elseif op == 'lmark' then
    redis.call('LSET', key, tonumber(a), tomb)
    marked = true
  elseif op == 'rpush' then redis.call('RPUSH', key, a)
  elseif op == 'sadd' then redis.call('SADD', key, a)
  elseif op == 'srem' then redis.call('SREM', key, a)
  elseif op == 'zadd' then redis.call('ZADD', key, a, b)
  elseif op == 'zrem' then redis.call('ZREM', key, a)
  elseif op == 'xadd' then redis.call('XADD', key, '*', a, b)
  elseif op == 'expire' then
    redis.call('EXPIRE', key, a)
    ttl = -1
  else
    written = written - 1
  end
  written = written + 1
end
if marked then redis.call('LREM', key, 0, tomb) end
if ttl > 0 and redis.call('PTTL', key) == -1 then redis.call('PEXPIRE', key, ttl) end
return written
