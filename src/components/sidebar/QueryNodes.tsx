// 对象树里的「查询」分组:这个库(PostgreSQL 是 schema)下保存的查询,双击打开。
import { useQueryClient } from "@tanstack/react-query";
import { FileCode, FileCode2, FilePlus2, FolderOpen, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";
import { savedQueryKeys, useSavedQueries } from "../../hooks/useSavedQueries";
import { useQueryTabs } from "../../hooks/useQueryTabs";
import { getBackend } from "../../lib/api";
import { formatWhen } from "../../lib/format";
import type { SavedQueryRef } from "../../lib/queryTypes";
import { errorMessage, type ConnectionView } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { groupKey, savedQueryKey } from "./treeKeys";
import { useTreeMenu } from "./treeMenu";
import { TreeRow, TreeStatusRow } from "./TreeRow";

interface QueryGroupProps {
  connection: ConnectionView;
  database: string;
  schema: string | null;
  depth: number;
}

export function SavedQueryGroup({ connection, database, schema, depth }: QueryGroupProps) {
  const query = useSavedQueries(connection.id, database, schema, true);
  const queryClient = useQueryClient();
  const { openNewQuery, openSavedQuery } = useQueryTabs();
  const openMenu = useTreeMenu();
  const [deleting, setDeleting] = useState<SavedQueryRef | null>(null);
  const items = query.data ?? [];
  const newHere = () => void openNewQuery(connection, { database, schema });
  const refresh = () =>
    void queryClient.invalidateQueries({ queryKey: savedQueryKeys.list(connection.id, database, schema) });

  return (
    <>
      <TreeRow
        nodeKey={groupKey(connection.id, database, schema, "queries")}
        depth={depth}
        label="查询"
        icon={<FileCode2 />}
        count={query.data ? items.length : undefined}
        expandable={items.length > 0 || Boolean(query.error)}
        onActivate={items.length > 0 || query.error ? undefined : newHere}
        onContextMenu={(event) =>
          openMenu(event, [
            { label: "新建查询", icon: <FilePlus2 />, shortcut: "⌘T", onSelect: newHere },
            { label: "刷新", icon: <RefreshCw />, onSelect: refresh },
          ])
        }
      >
        {query.error ? (
          <TreeStatusRow depth={depth + 1} status="error" message={errorMessage(query.error)} onRetry={refresh} />
        ) : (
          items.map((item) => {
            const ref: SavedQueryRef = { connectionId: connection.id, database, schema, name: item.name };
            const openIt = () => void openSavedQuery(connection, ref);
            return (
              <TreeRow
                key={item.name}
                nodeKey={savedQueryKey(connection.id, database, schema, item.name)}
                depth={depth + 1}
                label={item.name}
                title={item.modifiedAt ? `修改于 ${formatWhen(item.modifiedAt)}` : undefined}
                icon={<FileCode />}
                onActivate={openIt}
                onContextMenu={(event) =>
                  openMenu(event, [
                    { label: "打开", icon: <FolderOpen />, shortcut: "↩", onSelect: openIt },
                    "separator",
                    { label: "删除…", icon: <Trash2 />, danger: true, onSelect: () => setDeleting(ref) },
                  ])
                }
              />
            );
          })
        )}
      </TreeRow>
      {deleting && <DeleteQueryDialog query={deleting} onDone={() => setDeleting(null)} />}
    </>
  );
}

function DeleteQueryDialog({ query, onDone }: { query: SavedQueryRef; onDone(): void }) {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const confirm = async () => {
    setBusy(true);
    try {
      await (await getBackend()).deleteSavedQuery(query);
      await queryClient.invalidateQueries({
        queryKey: savedQueryKeys.list(query.connectionId, query.database, query.schema),
      });
      onDone();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };
  return (
    <Dialog
      open
      title={`删除查询「${query.name}」？`}
      width={420}
      onClose={busy ? () => {} : onDone}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="secondary" disabled={busy} onClick={onDone}>
            取消
          </Button>
          <Button variant="danger" busy={busy} onClick={() => void confirm()}>
            删除
          </Button>
        </>
      }
    >
      <p className="discard-message">保存的 SQL 文件会被删掉，删除后找不回来。已经打开的标签页不受影响。</p>
      {error && <p className="test-result is-error">{error}</p>}
    </Dialog>
  );
}
