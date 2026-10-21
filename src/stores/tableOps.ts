// 删除表、复制表的对话框开着哪一个。对象树右键打开,对话框挂在应用最外层。
import { create } from "zustand";
import type { ConnectionView, TableRef } from "../lib/types";

export type TableOpsDialog = { kind: "drop" | "copy"; connection: ConnectionView; table: TableRef };

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
