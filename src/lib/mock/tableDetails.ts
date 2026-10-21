// mock 后端的信息面板:属性按各家真实返回的那几项编,值取自对象列表的种子数据。
import type { DriverKind, PropertyValue, TableDetails, TableInfo, TableProperty } from "../types";

const text = (value: string | null): PropertyValue | null => (value ? { kind: "text", value } : null);
const count = (value: number | null): PropertyValue | null => (value === null ? null : { kind: "count", value });
const bytes = (value: number | null): PropertyValue | null => (value === null ? null : { kind: "bytes", value });
const time = (value: string | null): PropertyValue | null => (value ? { kind: "dateTime", value } : null);
const row = (label: string, value: PropertyValue | null): TableProperty => ({ label, value });

function properties(driver: DriverKind, info: TableInfo): TableProperty[] {
  const view = info.kind !== "table";
  if (driver === "sqlite") {
    return [row("类型", text(view ? "视图" : "表")), ...(view ? [] : [row("行", count(info.rows ?? 0))])];
  }
  if (driver === "postgres") {
    return [
      row("类型", text(view ? "视图" : "表")),
      ...(view
        ? []
        : [
            row("行（估算）", count(info.rows)),
            row("总大小", bytes((info.dataBytes ?? 0) + (info.indexBytes ?? 0))),
            row("数据大小", bytes(info.dataBytes)),
            row("索引大小", bytes(info.indexBytes)),
          ]),
      row("所有者", text("folio")),
      row("注释", text(info.comment)),
    ];
  }
  if (view) return [row("类型", text("视图")), row("定义者", text("root@%")), row("可更新", text("是"))];
  return [
    row("行（估算）", count(info.rows)),
    row("数据长度", bytes(info.dataBytes)),
    row("引擎", text(info.engine)),
    row("创建日期", time(info.createdAt)),
    row("修改日期", time(info.updatedAt)),
    row("排序规则", text("utf8mb4_general_ci")),
    row("行格式", text("Dynamic")),
    row("索引长度", bytes(info.indexBytes)),
    row("自动递增", count(info.rows === null ? null : info.rows + 1)),
    row("注释", text(info.comment)),
  ];
}

export function mockTableDetails(driver: DriverKind, info: TableInfo, statements: string[]): TableDetails {
  return {
    properties: properties(driver, info),
    ddl: statements.map((s) => `${s.replace(/;\s*$/, "")};`).join("\n\n"),
    ddlGenerated: driver === "postgres",
  };
}
