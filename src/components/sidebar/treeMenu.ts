import { createContext, useContext, type MouseEvent } from "react";
import type { MenuEntry } from "../ui/ContextMenu";

export type OpenTreeMenu = (event: MouseEvent, items: MenuEntry[]) => void;

export const TreeMenuContext = createContext<OpenTreeMenu>(() => {});

export const useTreeMenu = () => useContext(TreeMenuContext);
