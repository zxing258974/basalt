// 表操作的交互测试共用:在对象树里展开到某张表,右键选一项菜单。
import { act, screen, within } from "@testing-library/react";
import type { Backend } from "../lib/api";
import { createMockBackend } from "../lib/mockBackend";
import { renderApp } from "./renderApp";

export const tree = () => within(screen.getByRole("tree", { name: "连接" }));

/** 树节点那一行(treeitem 的第一个子元素),右键、双击都点它。 */
export const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;

export interface TreeTarget {
  /** 从分组到连接,依次双击展开。 */
  path: string[];
  database: string;
  group: "表" | "视图";
  table: string;
}

export const MYSQL_CUSTOMERS: TreeTarget = {
  path: ["开发", "本地 MySQL"],
  database: "shop",
  group: "表",
  table: "customers",
};
export const SQLITE_NOTES: TreeTarget = { path: ["笔记"], database: "main", group: "表", table: "notes" };

/** 渲染应用、执行 `before`(如先开好标签页),展开到 `target`,右键选 `item`。 */
export async function openTableMenu(
  item: string,
  target: TreeTarget = MYSQL_CUSTOMERS,
  backend: Backend = createMockBackend(),
  before: () => void = () => undefined,
) {
  const app = renderApp(backend);
  act(before);
  const { user } = app;
  for (const name of target.path) await user.dblClick(await findRow(name));
  await user.click(await findRow(target.database));
  await user.keyboard("{ArrowRight}");
  await user.dblClick(await findRow(target.group));
  await user.pointer({ keys: "[MouseRight]", target: await findRow(target.table) });
  await user.click(screen.getByRole("menuitem", { name: item }));
  return app;
}
