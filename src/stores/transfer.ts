// 导入导出对话框开着哪一个。对象树右键、表数据页、查询结果都从这里打开,对话框挂在应用最外层。
import { create } from "zustand";
import type { ExportTarget } from "../components/transfer/exportModel";
import type { ImportTarget } from "../components/transfer/importModel";

export type TransferDialog = { kind: "export"; target: ExportTarget } | { kind: "import"; target: ImportTarget };

interface TransferStore {
  dialog: TransferDialog | null;
  openExport(target: ExportTarget): void;
  openImport(target: ImportTarget): void;
  close(): void;
}

export const useTransfer = create<TransferStore>((set) => ({
  dialog: null,
  openExport: (target) => set({ dialog: { kind: "export", target } }),
  openImport: (target) => set({ dialog: { kind: "import", target } }),
  close: () => set({ dialog: null }),
}));
