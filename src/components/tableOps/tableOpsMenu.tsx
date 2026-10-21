// 对象树里表节点右键菜单的「复制表…」「删除表…」。
import { Copy, Trash2 } from "lucide-react";
import type { ConnectionView, TableRef } from "../../lib/types";
import { useTableOps } from "../../stores/tableOps";
import type { MenuItem } from "../ui/ContextMenu";

export function copyTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return {
    label: "复制表…",
    icon: <Copy />,
    onSelect: () => useTableOps.getState().open({ kind: "copy", connection, table }),
  };
}

export function dropTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return {
    label: "删除表…",
    icon: <Trash2 />,
    danger: true,
    onSelect: () => useTableOps.getState().open({ kind: "drop", connection, table }),
  };
}
