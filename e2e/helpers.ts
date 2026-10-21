// 截图测试和布局测试共用的步骤。
import { expect, type Page } from "@playwright/test";

/** 双击、右键这些事件挂在 treeitem 的第一行上。 */
export const row = (page: Page, name: string) =>
  page.getByRole("treeitem", { name, exact: true }).locator(":scope > .tree-row");

/** 页面不能比窗口宽:超出的部分在桌面窗口里看不到,也滚不过去。 */
export async function expectFitsWindow(page: Page) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow, "页面横向超出窗口的像素").toBeLessThanOrEqual(0);
}

export async function openWelcome(page: Page) {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /看清每一张表/ })).toBeVisible();
  await expect(page.getByRole("treeitem", { name: "笔记" })).toBeVisible();
}

/** 展开「开发 → 本地 MySQL → shop → 表」。 */
export async function expandShop(page: Page) {
  await row(page, "开发").dblclick();
  await row(page, "本地 MySQL").dblclick();
  await row(page, "shop").click();
  await page.keyboard.press("ArrowRight");
  await row(page, "表").dblclick();
  await expect(page.getByRole("treeitem", { name: "customers" })).toBeVisible();
}

/** 选中 shop.customers,打开右侧信息面板。 */
export async function inspectCustomers(page: Page) {
  await openWelcome(page);
  await expandShop(page);
  await row(page, "customers").click();
  const toggle = page.getByRole("banner").getByRole("button", { name: "信息" });
  if ((await toggle.getAttribute("aria-pressed")) !== "true") await toggle.click();
  const panel = page.getByRole("complementary", { name: "信息" });
  await expect(panel.getByRole("heading", { name: "customers" })).toBeVisible();
  await expect(panel.getByText("InnoDB")).toBeVisible();
  return panel;
}

/** 在 shop 上新建查询,打开 SQL 助手。 */
export async function askAssistant(page: Page) {
  await openWelcome(page);
  await row(page, "开发").dblclick();
  await row(page, "本地 MySQL").dblclick();
  await row(page, "shop").click({ button: "right" });
  await page.getByRole("menuitem", { name: /新建查询/ }).click();
  await page.getByRole("button", { name: "助手", exact: true }).click();
  const panel = page.getByRole("complementary", { name: "SQL 助手" });
  await expect(panel.getByText(/会附带「shop」里 \d+ 张表的结构/)).toBeVisible();
  return panel;
}
