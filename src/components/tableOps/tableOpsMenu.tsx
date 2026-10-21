// 对象树里表节点右键菜单的表操作:复制、改名、清空、截断、删除、转储、维护、复制表名。
import { ClipboardCopy, Copy, Eraser, FileCode, PenLine, Scissors, Trash2, Wrench } from "lucide-react";
import type { ConnectionView, TableRef } from "../../lib/types";
import { useTableOps, type TableOpsDialog } from "../../stores/tableOps";
import { useUi } from "../../stores/ui";
import type { MenuItem } from "../ui/ContextMenu";

const open = (dialog: TableOpsDialog) => () => useTableOps.getState().open(dialog);

export function copyTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return { label: "复制表…", icon: <Copy />, onSelect: open({ kind: "copy", connection, table }) };
}

export function renameTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return { label: "重命名…", icon: <PenLine />, onSelect: open({ kind: "rename", connection, table }) };
}

/** `rows`:对象列表里的行数(估算),确认框里写出来。 */
export function clearTableItem(connection: ConnectionView, table: TableRef, rows: number | null): MenuItem {
  return {
    label: "清空表…",
    icon: <Eraser />,
    danger: true,
    onSelect: open({ kind: "clear", mode: "delete", rows, connection, table }),
  };
}

export function truncateTableItem(connection: ConnectionView, table: TableRef, rows: number | null): MenuItem {
  return {
    label: "截断表…",
    icon: <Scissors />,
    danger: true,
    onSelect: open({ kind: "clear", mode: "truncate", rows, connection, table }),
  };
}

export function dropTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return {
    label: "删除表…",
    icon: <Trash2 />,
    danger: true,
    onSelect: open({ kind: "drop", connection, table }),
  };
}

export function dumpTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return { label: "转储 SQL 文件…", icon: <FileCode />, onSelect: open({ kind: "dump", connection, table }) };
}

export function maintainTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return { label: "维护…", icon: <Wrench />, onSelect: open({ kind: "maintain", connection, table }) };
}

/** 复制表(视图)名,不带库名和引号:贴进 SQL 里最常用的就是它。 */
export function copyNameItem(name: string, label = "复制表名"): MenuItem {
  return {
    label,
    icon: <ClipboardCopy />,
    onSelect: () => {
      const { notify } = useUi.getState();
      const clipboard = typeof navigator === "undefined" ? undefined : navigator.clipboard;
      if (!clipboard) {
        notify("没能复制：拿不到剪贴板。");
        return;
      }
      clipboard.writeText(name).then(
        () => notify(`已复制「${name}」。`),
        (error: unknown) => notify(`没能复制：${String(error)}`),
      );
    },
  };
}
