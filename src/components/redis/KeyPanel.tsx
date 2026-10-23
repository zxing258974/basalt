import { Clock, Cpu, Hash, Pencil, RefreshCw, Timer, Trash2 } from "lucide-react";
import { useCallback, useState } from "react";
import type { KeyDetail, KeyName } from "../../lib/redisTypes";
import { errorMessage, isCommandError } from "../../lib/types";
import { formatBytes } from "../transfer/exportModel";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import { DeleteKeyDialog, RenameKeyDialog, TtlDialog } from "./KeyDialogs";
import { KindBadge } from "./KindBadge";
import { formatLength, formatTtl } from "./redisKeyModel";
import { TableValue } from "./TableValue";
import { TextValue } from "./TextValue";
import { useKeyDetail, useRefreshAfterChange } from "./useRedisData";

interface KeyPanelProps {
  connectionId: string;
  db: number;
  keyId: string | null;
  active: boolean;
  /** 改名、删除之后选中的键跟着变。 */
  onKeyChange(key: string | null): void;
  /** 值里有几处没提交的修改(换键、关标签页前要确认)。 */
  onPendingChange(pending: number): void;
  /** 会丢掉没提交的修改的操作(改名、删除、重新读取)先经它确认;`verb` 写在确认框的按钮上。 */
  guard(action: () => void, verb: string): void;
}

type Open = "ttl" | "rename" | "delete" | null;

/** 右边:选中的那个键的类型、剩余时间、大小,和它的值。 */
export function KeyPanel(props: KeyPanelProps) {
  const { connectionId, db, keyId } = props;
  const query = useKeyDetail(connectionId, db, keyId);
  const refresh = useRefreshAfterChange(connectionId, db);
  const [open, setOpen] = useState<Open>(null);

  if (keyId === null) {
    return (
      <div className="key-panel">
        <EmptyState title="在左边选一个键" detail="选中后这里显示它的值；集合类的值可以像表格一样改。" />
      </div>
    );
  }
  const gone = isCommandError(query.error) && query.error.code === "not_found";
  // 后台重读失败一次(网络抖了)不把已经显示的值和正在改的东西换成错误页;键没了才换。
  if (query.error && (gone || !query.data)) {
    return (
      <div className="key-panel">
        <EmptyState
          title={gone ? "这个键已经不存在了" : "读取失败"}
          detail={gone ? "可能过期了，或者被别人删了。" : errorMessage(query.error)}
          tone={gone ? undefined : "error"}
          action={
            <Button
              onClick={() => {
                if (gone) props.onKeyChange(null);
                void refresh(keyId);
              }}
            >
              {gone ? "刷新列表" : "重试"}
            </Button>
          }
        />
      </div>
    );
  }
  const detail = query.data;
  if (!detail) {
    return (
      <div className="key-panel">
        <EmptyState title="正在读取…" />
      </div>
    );
  }
  const target = { connectionId, db };
  const afterChange = () => refresh(keyId);

  return (
    <div className="key-panel">
      <header className="key-head">
        <div className="key-head-title">
          <KindBadge kind={detail.kind} size="md" />
          <h2 className="key-name mono selectable">{detail.key.name}</h2>
        </div>
        <div className="key-head-actions">
          <Button
            variant="ghost"
            size="sm"
            icon={<RefreshCw />}
            aria-label="重新读取这个键"
            busy={query.isFetching}
            onClick={() => props.guard(() => void query.refetch(), "重新读取")}
          />
          <Button variant="ghost" size="sm" icon={<Timer />} onClick={() => setOpen("ttl")}>
            过期时间
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<Pencil />}
            onClick={() => props.guard(() => setOpen("rename"), "改名")}
          >
            改名
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<Trash2 />}
            className="is-danger"
            onClick={() => props.guard(() => setOpen("delete"), "删除")}
          >
            删除
          </Button>
        </div>
        <KeyFacts detail={detail} />
      </header>

      <KeyValueView {...props} detail={detail} onChanged={afterChange} />

      {open === "ttl" && (
        <TtlDialog target={target} detail={detail} onDone={afterChange} onClose={() => setOpen(null)} />
      )}
      {open === "rename" && (
        <RenameKeyDialog
          target={target}
          detail={detail}
          onDone={async (renamed: KeyName) => {
            props.onKeyChange(renamed.id);
            await refresh();
          }}
          onClose={() => setOpen(null)}
        />
      )}
      {open === "delete" && (
        <DeleteKeyDialog
          target={target}
          detail={detail}
          onDone={async () => {
            props.onKeyChange(null);
            await refresh();
          }}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  );
}

function KeyFacts({ detail }: { detail: KeyDetail }) {
  const length = formatLength(detail.kind, detail.length);
  return (
    <dl className="key-facts">
      <div className={detail.ttlMs === null ? "key-fact" : "key-fact is-expiring"}>
        <dt>
          <Clock aria-hidden />
          过期
        </dt>
        <dd>{detail.ttlMs === null ? "不过期" : `还剩 ${formatTtl(detail.ttlMs)}`}</dd>
      </div>
      {length && (
        <div className="key-fact">
          <dt>
            <Hash aria-hidden />
            大小
          </dt>
          <dd>{length}</dd>
        </div>
      )}
      {detail.memory !== null && (
        <div className="key-fact">
          <dt>
            <Cpu aria-hidden />
            内存
          </dt>
          <dd title={`MEMORY USAGE：${detail.memory.toLocaleString()} 字节`}>{formatBytes(detail.memory)}</dd>
        </div>
      )}
      {detail.encoding && (
        <div className="key-fact">
          <dt>编码</dt>
          <dd className="mono">{detail.encoding}</dd>
        </div>
      )}
    </dl>
  );
}

function KeyValueView(props: KeyPanelProps & { detail: KeyDetail; onChanged(): Promise<void> }) {
  const { detail, connectionId, db, onPendingChange } = props;
  const onEditingChange = useCallback((editing: boolean) => onPendingChange(editing ? 1 : 0), [onPendingChange]);
  const value = detail.value;
  switch (value.kind) {
    case "text":
      return (
        <TextValue
          key={detail.key.id}
          connectionId={connectionId}
          db={db}
          keyId={detail.key.id}
          text={value.text}
          truncated={value.truncated}
          active={props.active}
          onSaved={props.onChanged}
          onEditingChange={onEditingChange}
        />
      );
    case "bytes":
      return (
        <div className="key-value">
          <p className="key-value-note">不是 UTF-8 文本，按十六进制显示，不能在这里改。</p>
          <pre className="inspector-value text-value-view mono selectable" aria-label="字符串的值" tabIndex={0}>
            0x{value.hex}
          </pre>
        </div>
      );
    case "table":
      return (
        <TableValue
          key={detail.key.id}
          connectionId={connectionId}
          db={db}
          detail={detail}
          page={value.page}
          truncated={value.truncated}
          active={props.active}
          onCommitted={props.onChanged}
          onPendingChange={onPendingChange}
        />
      );
    case "unsupported":
      return (
        <div className="key-value">
          <EmptyState title="显示不了这种类型" detail={value.reason} />
        </div>
      );
  }
}
