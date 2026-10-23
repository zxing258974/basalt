// 右侧信息面板开没开、看的是选中格子的值、属性还是建表语句。只是个人偏好:读写本机存储失败就用默认值。
import { create } from "zustand";

export type InspectorTab = "value" | "properties" | "ddl";

const TABS: readonly InspectorTab[] = ["value", "properties", "ddl"];

const OPEN_KEY = "basalt.inspector.open";
const TAB_KEY = "basalt.inspector.tab";
/** 第一次打开时,窗口够宽才默认展开:窄窗口里先把地方留给表格。 */
const WIDE_ENOUGH = 1200;

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function save(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch (error) {
    console.warn("无法保存信息面板的设置", error);
  }
}

function initialOpen(): boolean {
  const saved = read(OPEN_KEY);
  if (saved !== null) return saved === "1";
  return typeof window !== "undefined" && window.innerWidth >= WIDE_ENOUGH;
}

interface InspectorStore {
  open: boolean;
  tab: InspectorTab;
  toggle(): void;
  setTab(tab: InspectorTab): void;
  /** 打开面板,看选中格子的值(表格右键「在右侧查看完整内容」)。 */
  showValue(): void;
}

function savedTab(): InspectorTab {
  const saved = read(TAB_KEY);
  return TABS.find((tab) => tab === saved) ?? "properties";
}

export const useInspector = create<InspectorStore>((set) => ({
  open: initialOpen(),
  tab: savedTab(),
  toggle: () =>
    set((s) => {
      save(OPEN_KEY, s.open ? "0" : "1");
      return { open: !s.open };
    }),
  setTab: (tab) => {
    save(TAB_KEY, tab);
    set({ tab });
  },
  showValue: () => {
    save(OPEN_KEY, "1");
    save(TAB_KEY, "value");
    set({ open: true, tab: "value" });
  },
}));
