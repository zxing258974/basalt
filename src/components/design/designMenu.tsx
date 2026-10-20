// 对象树右键菜单里和表结构设计有关的两项。
import { Grid2x2Plus, PencilRuler } from "lucide-react";
import { newTableTab, tableDesignTab, useTabs } from "../../stores/tabs";
import type { MenuItem } from "../ui/ContextMenu";

/** 表节点:打开这张表的设计器。 */
export function designTableItem(
  connectionId: string,
  database: string,
  schema: string | null,
  table: string,
): MenuItem {
  return {
    label: "设计表",
    icon: <PencilRuler />,
    onSelect: () => useTabs.getState().open(tableDesignTab(connectionId, database, schema, table)),
  };
}

/** 库(PostgreSQL 是 schema)节点:在这里新建一张表。 */
export function newTableItem(connectionId: string, database: string, schema: string | null): MenuItem {
  return {
    label: "新建表",
    icon: <Grid2x2Plus />,
    onSelect: () => useTabs.getState().open(newTableTab(connectionId, database, schema)),
  };
}
