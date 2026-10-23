// 标签页里的组件知道自己在哪个标签页、它是不是正显示着,不用一层层传 props。
import { createContext } from "react";

export interface TabScopeValue {
  tabId: string;
  active: boolean;
}

/** 不在标签页里(单独渲染的测试)为 null。 */
export const TabScope = createContext<TabScopeValue | null>(null);
