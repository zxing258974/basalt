// 连接的写操作:打开 / 关闭 / 复制 / 删除 / 记住密码,统一处理缓存失效、标签页与树的收起。
import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { needsPasswordPrompt } from "../components/connection/driverMeta";
import { connectionKey, folderKey } from "../components/sidebar/treeKeys";
import { getBackend } from "../lib/api";
import { errorMessage, isCommandError, type ConnectionConfig } from "../lib/types";
import { guardDiscard } from "../stores/discard";
import { useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";
import { catalogKeys, connectionsQuery } from "./useCatalog";
import { platform } from "../lib/platform";

/** 删连接不弹钥匙串授权框;密码删不掉时如实说一声。 */
export function passwordLeftNotice(): string {
  return platform() === "mac"
    ? "连接已删除，但密码还留在钥匙串里（多半是旧版 Basalt 存的，删除要授权）；可在「钥匙串访问」里搜索 dev.basalt.app 手动删。"
    : "连接已删除，但密码没能从 Windows 凭据管理器里删掉；可在「控制面板 → 凭据管理器 → Windows 凭据」里找 dev.basalt.app 手动删。";
}

export function useConnectionActions() {
  const queryClient = useQueryClient();
  const closeConnectionTabs = useTabs((s) => s.closeConnection);
  const { askPassword, setExpanded, collapseUnder, setConnectionError, notify } = useUi.getState();

  const refreshConnections = useCallback(
    () => queryClient.invalidateQueries({ queryKey: catalogKeys.connections }),
    [queryClient],
  );

  /** 在树上展开这个连接,连同它所在的分组文件夹。 */
  const reveal = useCallback(
    (connection: ConnectionConfig) => {
      setExpanded(connectionKey(connection.id), true);
      if (connection.group) setExpanded(folderKey(connection.group), true);
    },
    [setExpanded],
  );

  /**
   * 会话没了之后的界面清理:关掉它的标签页、收起子树、丢掉元数据缓存。
   * 不是用户主动关连接(改了连接配置、删了连接)时留下查询标签页,里面没保存的 SQL 不会悄悄没了。
   */
  const forget = useCallback(
    (id: string, keepQueries = false) => {
      closeConnectionTabs(id, keepQueries);
      collapseUnder(connectionKey(id));
      setConnectionError(id, null);
      queryClient.removeQueries({ queryKey: catalogKeys.connection(id) });
    },
    [closeConnectionTabs, collapseUnder, queryClient, setConnectionError],
  );

  /**
   * 已打开则什么都不做;需要密码时弹出密码框,由密码框完成打开。
   * `prompt: false`(启动时自动重连):要密码或密码不对时不弹框,直接返回 false。
   */
  const open = useCallback(
    async (
      connection: ConnectionConfig & { isOpen?: boolean },
      password?: string,
      { prompt = true }: { prompt?: boolean } = {},
    ) => {
      if (connection.isOpen) return true;
      if (password === undefined && needsPasswordPrompt(connection)) {
        if (prompt) askPassword({ connection });
        return false;
      }
      try {
        const server = await (await getBackend()).openConnection(connection.id, password);
        queryClient.setQueryData(catalogKeys.server(connection.id), server);
        await refreshConnections();
        reveal(connection);
        return true;
      } catch (error) {
        if (isCommandError(error) && error.code === "authentication") {
          if (prompt) askPassword({ connection, error: error.message });
          return false;
        }
        throw error;
      }
    },
    [askPassword, queryClient, refreshConnections, reveal],
  );

  /** 打开并把失败原因记到界面状态:节点照常展开,显示错误和「重试」。 */
  const openWithFeedback = useCallback(
    async (connection: ConnectionConfig & { isOpen?: boolean }) => {
      setConnectionError(connection.id, null);
      try {
        return await open(connection);
      } catch (error) {
        setConnectionError(connection.id, errorMessage(error));
        reveal(connection);
        return true;
      }
    },
    [open, reveal, setConnectionError],
  );

  /** 保存之后:如果后端因为连接参数变了而断开了会话,界面也跟着清理。 */
  const afterSave = useCallback(
    async (id: string) => {
      // 直接取最新列表:连接列表的查询未必挂载着,失效后读缓存可能读到旧值或空值。
      const connections = await queryClient.fetchQuery({ ...connectionsQuery, staleTime: 0 });
      if (!connections.find((c) => c.id === id)?.isOpen) forget(id, true);
    },
    [forget, queryClient],
  );

  const rememberPassword = useCallback(
    async (id: string, password: string) => {
      await (await getBackend()).rememberPassword(id, password);
      await refreshConnections();
    },
    [refreshConnections],
  );

  /** 关闭连接会关掉它的标签页:有未提交的表格修改先确认,取消就什么都不做。 */
  const close = useCallback(
    (id: string) =>
      new Promise<void>((resolve, reject) => {
        const run = async () => {
          await (await getBackend()).closeConnection(id);
          forget(id);
          await refreshConnections();
        };
        guardDiscard(
          (tab) => tab.connectionId === id,
          () => {
            void run().then(resolve, reject);
          },
          resolve,
        );
      }),
    [forget, refreshConnections],
  );

  const duplicate = useCallback(
    async (id: string) => {
      const copy = await (await getBackend()).duplicateConnection(id);
      await refreshConnections();
      return copy;
    },
    [refreshConnections],
  );

  const remove = useCallback(
    async (id: string) => {
      const { passwordLeft } = await (await getBackend()).deleteConnection(id);
      forget(id, true);
      await refreshConnections();
      if (passwordLeft) notify(passwordLeftNotice());
    },
    [forget, refreshConnections, notify],
  );

  const refresh = useCallback(
    async (id?: string) => {
      if (id) await queryClient.invalidateQueries({ queryKey: catalogKeys.connection(id) });
      await refreshConnections();
    },
    [queryClient, refreshConnections],
  );

  /** 菜单、工具栏这类没有自己出错位置的操作:失败时在状态栏提示。 */
  const withNotice = useCallback(
    (label: string, action: () => Promise<unknown>) => {
      action().catch((error: unknown) => notify(`${label}失败：${errorMessage(error)}`));
    },
    [notify],
  );

  return { open, openWithFeedback, afterSave, rememberPassword, close, duplicate, remove, refresh, withNotice };
}
