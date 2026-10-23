import { useId, useState } from "react";
import { getBackend } from "../../lib/api";
import { REDIS_KINDS, type KeyDetail, type KeyName, type RedisKind } from "../../lib/redisTypes";
import { errorMessage } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field, SelectField } from "../ui/Field";
import { KIND_NAMES, formatTtl, parseTtl } from "./redisKeyModel";

interface Target {
  connectionId: string;
  db: number;
}

/** 提交时转圈、失败时把原因显示在框里;成功了交给调用方关框。 */
function useSubmit() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (failure) {
      setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, setError, run };
}

const PRESETS: readonly [string, number][] = [
  ["1 分钟", 60],
  ["1 小时", 3600],
  ["1 天", 86_400],
  ["7 天", 604_800],
];

export function TtlDialog({
  target,
  detail,
  onDone,
  onClose,
}: {
  target: Target;
  detail: KeyDetail;
  onDone(): Promise<void>;
  onClose(): void;
}) {
  const [text, setText] = useState(detail.ttlMs === null ? "" : String(Math.ceil(detail.ttlMs / 1000)));
  const { busy, error, run } = useSubmit();
  const parsed = parseTtl(text);
  const formId = useId();
  const submit = () =>
    void run(async () => {
      if (!parsed.ok) return;
      await (await getBackend()).redisSetTtl(target.connectionId, target.db, detail.key.id, parsed.seconds);
      await onDone();
      onClose();
    });
  return (
    <Dialog
      open
      title="过期时间"
      subtitle={`「${detail.key.name}」现在${detail.ttlMs === null ? "不过期" : `还剩 ${formatTtl(detail.ttlMs)}`}`}
      width={440}
      onClose={() => !busy && onClose()}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" type="submit" form={formId} busy={busy} disabled={!parsed.ok}>
            {parsed.ok && parsed.seconds === null ? "设为不过期" : "设置"}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className="redis-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (parsed.ok) submit();
        }}
      >
        <Field
          label="从现在起多少秒后过期"
          value={text}
          inputMode="numeric"
          autoFocus
          hint="留空就是不过期"
          error={parsed.ok ? undefined : parsed.message}
          onValueChange={setText}
        />
        <div className="ttl-presets" role="group" aria-label="常用时长">
          {PRESETS.map(([label, seconds]) => (
            <Button key={label} variant="ghost" size="sm" onClick={() => setText(String(seconds))}>
              {label}
            </Button>
          ))}
        </div>
        {error && (
          <p className="field-error" role="alert">
            {error}
          </p>
        )}
      </form>
    </Dialog>
  );
}

export function RenameKeyDialog({
  target,
  detail,
  onDone,
  onClose,
}: {
  target: Target;
  detail: KeyDetail;
  onDone(renamed: KeyName): Promise<void>;
  onClose(): void;
}) {
  const [name, setName] = useState(detail.key.binary ? "" : detail.key.name);
  const { busy, error, run } = useSubmit();
  const formId = useId();
  const submit = () =>
    void run(async () => {
      const renamed = await (await getBackend()).redisRenameKey(target.connectionId, target.db, detail.key.id, name);
      await onDone(renamed);
      onClose();
    });
  return (
    <Dialog
      open
      title="改名"
      subtitle="新名字已经有键时不改，不会覆盖它。值和剩余时间不变。"
      width={480}
      onClose={() => !busy && onClose()}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button
            variant="primary"
            type="submit"
            form={formId}
            busy={busy}
            disabled={!name || name === detail.key.name}
          >
            改名
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className="redis-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (name && name !== detail.key.name) submit();
        }}
      >
        <Field label="新的键名" value={name} autoFocus className="mono" onValueChange={setName} />
        {error && (
          <p className="field-error" role="alert">
            {error}
          </p>
        )}
      </form>
    </Dialog>
  );
}

export function DeleteKeyDialog({
  target,
  detail,
  onDone,
  onClose,
}: {
  target: Target;
  detail: KeyDetail;
  onDone(): Promise<void>;
  onClose(): void;
}) {
  const { busy, error, run } = useSubmit();
  return (
    <Dialog
      open
      title={`删除「${detail.key.name}」？`}
      subtitle="删除之后不能恢复。大键在服务器后台释放（UNLINK），不会卡住服务器。"
      width={460}
      onClose={() => !busy && onClose()}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button
            variant="danger"
            busy={busy}
            autoFocus
            onClick={() =>
              void run(async () => {
                await (await getBackend()).redisDeleteKeys(target.connectionId, target.db, [detail.key.id]);
                await onDone();
                onClose();
              })
            }
          >
            删除
          </Button>
        </>
      }
    >
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}

/** 每种类型第一项要填什么。 */
const FIRST_ITEM: Record<string, { field?: string; value: string; score?: true }> = {
  string: { value: "值" },
  hash: { field: "第一个字段", value: "它的值" },
  list: { value: "第一个元素" },
  set: { value: "第一个成员" },
  zset: { value: "第一个成员", score: true },
  stream: { field: "第一条消息的字段", value: "它的值" },
};

export function NewKeyDialog({
  target,
  prefix,
  onDone,
  onClose,
}: {
  target: Target;
  /** 选中的分组:新键名先填上它,比如 `user:`。 */
  prefix: string;
  onDone(created: KeyName): Promise<void>;
  onClose(): void;
}) {
  const [name, setName] = useState(prefix);
  const [kind, setKind] = useState<RedisKind>("string");
  const [ttl, setTtl] = useState("");
  const [field, setField] = useState("");
  const [value, setValue] = useState("");
  const [score, setScore] = useState("0");
  const { busy, error, run } = useSubmit();
  const parsedTtl = parseTtl(ttl);
  const first = FIRST_ITEM[kind]!;
  const ready = name !== "" && parsedTtl.ok && (!first.field || field !== "");
  const formId = useId();
  const submit = () =>
    void run(async () => {
      if (!parsedTtl.ok) return;
      const created = await (
        await getBackend()
      ).redisCreateKey(target.connectionId, target.db, {
        name,
        kind,
        ttlSeconds: parsedTtl.seconds,
        field: first.field ? field : null,
        value,
        score: first.score ? score : null,
      });
      await onDone(created);
      onClose();
    });
  return (
    <Dialog
      open
      title="新建键"
      subtitle={`建在 db${target.db} 里；同名的键已经有了就不建。`}
      width={520}
      onClose={() => !busy && onClose()}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" type="submit" form={formId} busy={busy} disabled={!ready}>
            新建
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className="redis-form new-key-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) submit();
        }}
      >
        <Field label="键名" value={name} autoFocus className="mono" onValueChange={setName} />
        <SelectField
          label="类型"
          value={kind}
          options={REDIS_KINDS.map((k) => ({ value: k, label: `${k}（${KIND_NAMES[k]}）` }))}
          onValueChange={(next) => setKind(next)}
        />
        {first.field && <Field label={first.field} value={field} className="mono" onValueChange={setField} />}
        <Field label={first.value} value={value} className="mono" onValueChange={setValue} />
        {first.score && <Field label="分数" value={score} inputMode="decimal" onValueChange={setScore} />}
        <Field
          label="过期秒数"
          value={ttl}
          inputMode="numeric"
          hint="留空就是不过期"
          error={parsedTtl.ok ? undefined : parsedTtl.message}
          onValueChange={setTtl}
        />
        {error && (
          <p className="field-error" role="alert">
            {error}
          </p>
        )}
      </form>
    </Dialog>
  );
}
