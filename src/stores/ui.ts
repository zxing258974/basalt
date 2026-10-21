import { create } from "zustand";
import type { ConnectionConfig, DriverKind } from "../lib/types";

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
  error?: string;
}

interface UiStore {
  expanded: Readonly<Record<string, boolean>>;
  selectedKey: string | null;
  connectionDialog: ConnectionDialogState;
  passwordPrompt: PasswordPromptState | null;
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
