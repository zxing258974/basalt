// 表操作的对话框开着哪一个。对象树右键打开,对话框挂在应用最外层。
import { create } from "zustand";
import type { ClearMode, ConnectionView, TableRef } from "../lib/types";

interface DialogTarget {
  connection: ConnectionView;
  table: TableRef;
}

export type TableOpsDialog =
  | ({ kind: "drop" | "copy" | "rename" | "dump" | "maintain" } & DialogTarget)
  /** 清空、截断是同一个对话框,`mode` 是打开时选中的做法。 */
  | ({ kind: "clear"; mode: ClearMode } & DialogTarget);

interface TableOpsStore {
  dialog: TableOpsDialog | null;
  open(dialog: TableOpsDialog): void;
  close(): void;
}

export const useTableOps = create<TableOpsStore>((set) => ({
  dialog: null,
  open: (dialog) => set({ dialog }),
  close: () => set({ dialog: null }),
}));
