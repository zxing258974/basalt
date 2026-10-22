import { create } from "zustand";
import type { ConnectionConfig, DriverKind, HostKey } from "../lib/types";

export type ConnectionDialogState =
  { mode: "closed" } | { mode: "create"; driver: DriverKind | null } | { mode: "edit"; connection: ConnectionConfig };

/** 状态栏右侧的一行提示,用于没有专属位置显示的操作失败。 */
export interface Notice {
  id: number;
  message: string;
}

/** 关闭标签页或连接会丢掉未提交的修改时,先问一句。 */
export interface DiscardPrompt {
  /** 表格里未提交的修改行数。 */
  count: number;
  /** 没保存的查询标签页个数。 */
  queries: number;
  /** 表结构没保存的设计器个数。 */
  designs: number;
  /** 是退出整个应用(⌘Q、关主窗口),不是关标签页或连接。 */
  quit?: boolean;
  /** 不是关闭,而是这个操作让修改没了(如「清空」「改名」):确认框照这个说。 */
  action?: string;
  onConfirm(): void;
  onCancel?(): void;
}

export interface PasswordPromptState {
  connection: ConnectionConfig;
  /** 数据库密码不对:重新问数据库密码。 */
  error?: string;
  /** SSH 密码或私钥口令不对、或者后端说要:问 SSH 那一个。 */
  sshError?: string;
}

/** 第一次连接某台 SSH 服务器:等用户核对指纹。 */
export interface HostKeyPromptState {
  hostKey: HostKey;
  /** 等这个答案的调用方:同一台服务器同时开的几个连接共用一个框。 */
  resolvers: readonly ((trusted: boolean) => void)[];
}

/** 同一台服务器出示的同一把钥匙。 */
function sameServerKey(a: HostKey, b: HostKey): boolean {
  return a.host.toLowerCase() === b.host.toLowerCase() && a.port === b.port && a.fingerprint === b.fingerprint;
}

interface UiStore {
  expanded: Readonly<Record<string, boolean>>;
  selectedKey: string | null;
  connectionDialog: ConnectionDialogState;
  passwordPrompt: PasswordPromptState | null;
  /** 正在显示的指纹确认框。 */
  hostKeyPrompt: HostKeyPromptState | null;
  /** 排着队的别的服务器,一个答完再弹下一个。 */
  hostKeyQueue: readonly HostKeyPromptState[];
  /** 弹出指纹确认框(或者排队),用户点了「信任」返回 true。 */
  confirmHostKey(hostKey: HostKey): Promise<boolean>;
  /** 指纹确认框的按钮。 */
  answerHostKey(trusted: boolean): void;
  pendingDelete: ConnectionConfig | null;
  discardPrompt: DiscardPrompt | null;
  askDiscard(prompt: DiscardPrompt | null): void;
  /** 最近一次打开失败的原因,显示在连接节点下方。 */
  connectionErrors: Readonly<Record<string, string>>;
  setConnectionError(id: string, message: string | null): void;
  notice: Notice | null;
  notify(message: string): void;
  /** 只关掉指定的那条,避免把后来的新提示一起关掉。 */
  dismissNotice(id: number): void;
  setExpanded(key: string, expanded: boolean): void;
  collapseUnder(prefix: string): void;
  select(key: string | null): void;
  openConnectionDialog(state: ConnectionDialogState): void;
  closeConnectionDialog(): void;
  askPassword(state: PasswordPromptState | null): void;
  confirmDelete(connection: ConnectionConfig | null): void;
}

export const useUi = create<UiStore>((set) => ({
  expanded: {},
  selectedKey: null,
  connectionDialog: { mode: "closed" },
  passwordPrompt: null,
  hostKeyPrompt: null,
  hostKeyQueue: [],
  confirmHostKey: (hostKey) =>
    new Promise<boolean>((resolve) => {
      set((s) => {
        const same = (prompt: HostKeyPromptState) => sameServerKey(prompt.hostKey, hostKey);
        const join = (prompt: HostKeyPromptState) => ({ ...prompt, resolvers: [...prompt.resolvers, resolve] });
        // 同一台服务器同时来的几个连接共用一个框,答一次都算数。
        if (s.hostKeyPrompt && same(s.hostKeyPrompt)) return { hostKeyPrompt: join(s.hostKeyPrompt) };
        if (s.hostKeyQueue.some(same)) {
          return { hostKeyQueue: s.hostKeyQueue.map((prompt) => (same(prompt) ? join(prompt) : prompt)) };
        }
        const entry = { hostKey, resolvers: [resolve] };
        // 别的服务器的排在后面,一个答完再弹下一个,不把前一个当成「不信任」了结。
        return s.hostKeyPrompt ? { hostKeyQueue: [...s.hostKeyQueue, entry] } : { hostKeyPrompt: entry };
      });
    }),
  answerHostKey: (trusted) =>
    set((s) => {
      s.hostKeyPrompt?.resolvers.forEach((resolve) => resolve(trusted));
      const [next = null, ...rest] = s.hostKeyQueue;
      return { hostKeyPrompt: next, hostKeyQueue: rest };
    }),
  pendingDelete: null,
  discardPrompt: null,
  askDiscard: (discardPrompt) => set({ discardPrompt }),
  connectionErrors: {},
  notice: null,
  notify: (message) => set((s) => ({ notice: { id: (s.notice?.id ?? 0) + 1, message } })),
  dismissNotice: (id) => set((s) => (s.notice?.id === id ? { notice: null } : s)),
  setConnectionError: (id, message) =>
    set((s) => {
      const { [id]: _previous, ...rest } = s.connectionErrors;
      return { connectionErrors: message === null ? rest : { ...rest, [id]: message } };
    }),
  setExpanded: (key, expanded) => set((s) => ({ expanded: { ...s.expanded, [key]: expanded } })),
  // 连接关闭时把它下面所有展开的节点一起收起。
  collapseUnder: (prefix) =>
    set((s) => ({
      expanded: Object.fromEntries(Object.entries(s.expanded).filter(([key]) => !key.startsWith(prefix))),
    })),
  select: (key) => set({ selectedKey: key }),
  openConnectionDialog: (connectionDialog) => set({ connectionDialog }),
  closeConnectionDialog: () => set({ connectionDialog: { mode: "closed" } }),
  askPassword: (passwordPrompt) => set({ passwordPrompt }),
  confirmDelete: (pendingDelete) => set({ pendingDelete }),
}));
