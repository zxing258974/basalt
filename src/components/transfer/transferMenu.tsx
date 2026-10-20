// 对象树右键菜单里和导入导出有关的两项。
import { FileDown, FileUp } from "lucide-react";
import type { ConnectionView, TableRef } from "../../lib/types";
import { useTransfer } from "../../stores/transfer";
import type { MenuItem } from "../ui/ContextMenu";

function title(table: TableRef): string {
  return [table.database, table.schema, table.table].filter(Boolean).join(".");
}

/** 表、视图节点:导出这张表的全部数据。 */
export function exportTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return {
    label: "导出数据…",
    icon: <FileDown />,
    onSelect: () =>
      useTransfer.getState().openExport({
        connectionId: connection.id,
        driver: connection.driver,
        title: title(table),
        baseName: table.table,
        scope: { kind: "table", table, filter: null, sort: null },
      }),
  };
}

/** 表节点:从 CSV / Excel 导入。 */
export function importTableItem(connection: ConnectionView, table: TableRef): MenuItem {
  return {
    label: "导入数据…",
    icon: <FileUp />,
    onSelect: () =>
      useTransfer
        .getState()
        .openImport({ connectionId: connection.id, driver: connection.driver, table, title: title(table) }),
  };
}
