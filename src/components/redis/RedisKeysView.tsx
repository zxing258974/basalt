import { useCallback, useEffect, useState } from "react";
import { useConnections, useRedisKeyspace } from "../../hooks/useCatalog";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { guardDiscard } from "../../stores/discard";
import { useTabs, type RedisKeysTab } from "../../stores/tabs";
import { DriverBadge } from "../connection/DriverBadge";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import { KeyBrowser } from "./KeyBrowser";
import { NewKeyDialog } from "./KeyDialogs";
import { KeyPanel } from "./KeyPanel";
import { useRefreshAfterChange } from "./useRedisData";

/** Redis 一个库:左边键列表,右边选中的键。 */
export function RedisKeysView({ tab, active }: { tab: RedisKeysTab; active: boolean }) {
  const connection = useConnections().data?.find((c) => c.id === tab.connectionId);
  const isOpen = Boolean(connection?.isOpen);
  const { openWithFeedback } = useConnectionActions();
  const keyspace = useRedisKeyspace(tab.connectionId, isOpen).data;
  const refresh = useRefreshAfterChange(tab.connectionId, tab.db);
  const [selected, setSelected] = useState<string | null>(null);
  const [creating, setCreating] = useState<string | null>(null);

  const setPending = useTabs((s) => s.setPending);
  const reportPending = useCallback((pending: number) => setPending(tab.id, pending), [setPending, tab.id]);
  useEffect(() => () => setPending(tab.id, 0), [setPending, tab.id]);

  /** 会丢掉当前键没提交的修改的操作:先问一句。 */
  const guard = useCallback(
    (action: () => void, verb: string) => guardDiscard((t) => t.id === tab.id, action, undefined, verb),
    [tab.id],
  );
  const select = (key: string | null) => {
    if (key !== selected) guard(() => setSelected(key), "换一个键");
  };

  const keys = keyspace?.databases.find((d) => d.index === tab.db)?.keys;

  return (
    <section className="redis-keys" aria-label={`${connection?.name ?? ""} db${tab.db} 的键`}>
      <header className="data-header">
        <div className="data-heading">
          {connection && <DriverBadge driver={connection.driver} />}
          <span className="data-crumb">{connection?.name ?? "已关闭的连接"}</span>
          <span className="data-crumb-sep">/</span>
          <h1 className="data-title">db{tab.db}</h1>
          {keys !== undefined && <span className="data-summary">{keys.toLocaleString()} 个键</span>}
        </div>
      </header>

      {!connection ? (
        <EmptyState title="这个连接已经不存在" />
      ) : !isOpen ? (
        <EmptyState
          title="连接已关闭"
          action={
            <Button variant="primary" onClick={() => void openWithFeedback(connection)}>
              重新打开
            </Button>
          }
        />
      ) : (
        <div className="redis-split">
          <KeyBrowser
            connectionId={tab.connectionId}
            db={tab.db}
            selected={selected}
            onSelect={select}
            onNewKey={setCreating}
          />
          <KeyPanel
            connectionId={tab.connectionId}
            db={tab.db}
            keyId={selected}
            active={active}
            onKeyChange={setSelected}
            onPendingChange={reportPending}
            guard={guard}
          />
        </div>
      )}

      {creating !== null && (
        <NewKeyDialog
          target={{ connectionId: tab.connectionId, db: tab.db }}
          prefix={creating}
          onDone={async (created) => {
            await refresh();
            select(created.id);
          }}
          onClose={() => setCreating(null)}
        />
      )}
    </section>
  );
}
