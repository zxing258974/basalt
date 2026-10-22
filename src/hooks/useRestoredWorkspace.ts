// 启动时把上次开着的标签页放回去,然后开始记录之后的变化(stores/workspace.ts)。
// 查询马上回来;表数据、对象列表、设计器要等连接打开,不用问密码就能连上的才自动连,
// 连不上或要输密码的就不恢复,在状态栏说一声。
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { needsPasswordPrompt } from "../components/connection/driverMeta";
import { getBackend } from "../lib/api";
import { errorMessage, isCommandError, type ConnectionView } from "../lib/types";
import {
  insertByOrder,
  parseWorkspace,
  planRestore,
  restoredTab,
  type RestorePlan,
  type SavedTab,
} from "../lib/workspace";
import { useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";
import { startRecording, useWorkspace } from "../stores/workspace";
import { connectionsQuery } from "./useCatalog";
import { useConnectionActions } from "./useConnectionActions";

type Open = ReturnType<typeof useConnectionActions>["open"];

/** 把一个连接的标签页按上次的位置插回去;上次停在其中一个、而用户还没点过别的标签页时,切回到它。 */
function putBack(tabs: readonly SavedTab[], plan: RestorePlan, fallbackActive: string | null): void {
  useTabs.setState((s) => {
    let next = s.tabs;
    for (const tab of tabs) {
      if (!next.some((t) => t.id === tab.id)) next = insertByOrder(next, restoredTab(tab), plan.order);
    }
    const backToLast = tabs.some((t) => t.id === plan.activeId) && s.activeId === fallbackActive;
    return { tabs: next, activeId: backToLast ? plan.activeId : s.activeId };
  });
}

function stopWaiting(connectionId: string): void {
  const { waiting, setWaiting } = useWorkspace.getState();
  setWaiting(waiting.filter((t) => t.connectionId !== connectionId));
}

/** 启动时没能自动连上的原因,说给用户听。 */
function restoreFailure(error: unknown): string {
  if (isCommandError(error)) {
    switch (error.code) {
      case "authentication":
        return "数据库密码不对";
      case "ssh_authentication":
        return "SSH 密码或私钥口令不对";
      case "ssh_secret_required":
        return "要输入 SSH 密码或私钥口令";
      case "host_key_unknown":
        return "第一次连这台 SSH 服务器，要先核对主机密钥";
      case "host_key_changed":
        return "SSH 服务器的主机密钥和记下的不一样，打开连接可以看到详情";
    }
  }
  return errorMessage(error);
}

async function reconnect(
  connection: ConnectionView,
  tabs: readonly SavedTab[],
  plan: RestorePlan,
  fallbackActive: string | null,
  open: Open,
  isCancelled: () => boolean,
): Promise<void> {
  let reason: string | null = null;
  if (needsPasswordPrompt(connection)) {
    reason = "要输入密码";
  } else {
    try {
      if (!(await open(connection, undefined, { prompt: false }))) reason = "要输入密码";
    } catch (error) {
      reason = restoreFailure(error);
    }
  }
  if (isCancelled()) return;
  if (reason === null) putBack(tabs, plan, fallbackActive);
  else
    useUi
      .getState()
      .notify(
        `上次开着的「${connection.name}」的 ${tabs.length} 个标签页没有恢复（${reason}），打开连接后可以从左侧重新打开。`,
      );
  stopWaiting(connection.id);
}

async function restore(open: Open, fetchConnections: () => Promise<ConnectionView[]>, isCancelled: () => boolean) {
  const text = await (await getBackend()).loadWorkspace();
  const saved = text === null ? null : parseWorkspace(text);
  // 恢复期间用户已经自己开了标签页:不去打乱,上次的就算了。
  if (!saved || isCancelled() || useTabs.getState().tabs.length > 0) return;
  const connections = await fetchConnections();
  if (isCancelled()) return;
  const plan = planRestore(saved, new Set(connections.map((c) => c.id)));
  const fallbackActive = plan.now.some((t) => t.id === plan.activeId) ? plan.activeId : (plan.now[0]?.id ?? null);
  useTabs.setState({ tabs: plan.now, activeId: fallbackActive });
  useWorkspace.getState().setWaiting([...plan.waiting.values()].flat(), plan.order);
  for (const [connectionId, tabs] of plan.waiting) {
    const connection = connections.find((c) => c.id === connectionId);
    if (connection) void reconnect(connection, tabs, plan, fallbackActive, open, isCancelled);
  }
}

export function useRestoredWorkspace(): void {
  const queryClient = useQueryClient();
  const { open } = useConnectionActions();
  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | null = null;
    const isCancelled = () => cancelled;
    const fetchConnections = () => queryClient.fetchQuery(connectionsQuery);
    restore(open, fetchConnections, isCancelled)
      .catch((error: unknown) => useUi.getState().notify(`没能恢复上次的标签页：${errorMessage(error)}`))
      .finally(() => {
        if (!cancelled) stop = startRecording();
      });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [open, queryClient]);
}
