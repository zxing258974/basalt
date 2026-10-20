// 关键画面的截图回归。宽度和深浅主题由 playwright.config.ts 里的 project 决定,
// 基准图在 e2e/__screenshots__/,界面有意改动后用 `pnpm test:visual:update` 重新生成。
import { expect, test, type Page } from "@playwright/test";

/** 双击、右键这些事件挂在 treeitem 的第一行上。 */
const row = (page: Page, name: string) =>
  page.getByRole("treeitem", { name, exact: true }).locator(":scope > .tree-row");

/** 鼠标停在哪个按钮上就会带上悬停样式,截图前挪到状态栏左下角。 */
async function parkPointer(page: Page) {
  const viewport = page.viewportSize();
  await page.mouse.move(1, (viewport?.height ?? 1) - 1);
}

async function snapshot(page: Page, name: string) {
  await parkPointer(page);
  await page.evaluate(() => document.fonts.ready);
  await expect(page).toHaveScreenshot(`${name}.png`);
}

async function openWelcome(page: Page) {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /看清每一张表/ })).toBeVisible();
  await expect(page.getByRole("treeitem", { name: "笔记" })).toBeVisible();
}

/** 展开「开发 → 本地 MySQL → shop → 表」。 */
async function expandShop(page: Page) {
  await row(page, "开发").dblclick();
  await row(page, "本地 MySQL").dblclick();
  await row(page, "shop").click();
  await page.keyboard.press("ArrowRight");
  await row(page, "表").dblclick();
  await expect(page.getByRole("treeitem", { name: "customers" })).toBeVisible();
}

test("welcome", async ({ page }) => {
  await openWelcome(page);
  await snapshot(page, "welcome");
});

test("object tree expanded", async ({ page }) => {
  await openWelcome(page);
  await expandShop(page);
  await snapshot(page, "object-tree");
});

test("object list", async ({ page }) => {
  await openWelcome(page);
  await expandShop(page);
  await row(page, "shop").dblclick();
  await expect(page.getByRole("cell", { name: "order_items" })).toBeVisible();
  await snapshot(page, "object-list");
});

test("new connection: pick a type", async ({ page }) => {
  await openWelcome(page);
  await page.getByRole("banner").getByRole("button", { name: "新建连接" }).click();
  await expect(page.getByRole("dialog", { name: "新建连接" })).toBeVisible();
  await snapshot(page, "new-connection-types");
});

test("new connection: MySQL form", async ({ page }) => {
  await openWelcome(page);
  await page.getByRole("banner").getByRole("button", { name: "新建连接" }).click();
  await page.getByRole("dialog").getByText("MySQL", { exact: true }).click();
  await expect(page.getByLabel("主机")).toHaveValue("127.0.0.1");
  await snapshot(page, "new-connection-form");
});
