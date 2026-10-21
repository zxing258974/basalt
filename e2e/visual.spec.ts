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

/** 页面不能比窗口宽:超出的部分在桌面窗口里看不到,也滚不过去。 */
async function expectFitsWindow(page: Page) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow, "页面横向超出窗口的像素").toBeLessThanOrEqual(0);
}

async function snapshot(page: Page, name: string) {
  await expectFitsWindow(page);
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

/** 在 shop 上右键「新建查询」,写三条语句后全部执行。 */
async function runShopQuery(page: Page) {
  await openWelcome(page);
  await row(page, "开发").dblclick();
  await row(page, "本地 MySQL").dblclick();
  await row(page, "shop").click({ button: "right" });
  await page.getByRole("menuitem", { name: /新建查询/ }).click();
  const editor = page.getByRole("textbox", { name: "查询 1 的 SQL" });
  await editor.click();
  await page.keyboard.type("SELECT * FROM customers LIMIT 30;\nUPDATE orders SET status = 'paid' WHERE id = 7;\n");
  await page.keyboard.type("SELECT * FROM missing;");
  // 点按钮执行,编辑器同时失去焦点,截图里不会有闪烁的光标。
  await page.getByRole("button", { name: "全部执行" }).click();
  await expect(page.getByText("第 3 条出错。")).toBeVisible();
}

test("query: messages after a failed statement", async ({ page }) => {
  await runShopQuery(page);
  await expect(page.getByRole("tab", { name: "消息" })).toHaveAttribute("aria-selected", "true");
  await snapshot(page, "query-messages");
});

test("query: result grid", async ({ page }) => {
  await runShopQuery(page);
  await page.getByRole("tab", { name: /结果 1/ }).click();
  await expect(page.getByRole("grid", { name: "查询 1 的结果 1" })).toBeVisible();
  await expect(page.getByRole("button", { name: "导出" })).toBeInViewport({ ratio: 1 });
  await snapshot(page, "query-results");
});

test("import: Navicat saved queries", async ({ page }) => {
  await openWelcome(page);
  await page.getByRole("banner").getByRole("button", { name: "导入" }).click();
  await page.getByRole("radio", { name: /Navicat 保存的查询.*检测到 3 条查询/ }).click();
  await expect(page.getByRole("row", { name: /核心期刊/ })).toBeVisible();
  await snapshot(page, "import-queries");
});

/** 打开 customers 的设计器,改一个字段名、加一个新字段。 */
async function designCustomers(page: Page) {
  await openWelcome(page);
  await expandShop(page);
  await row(page, "customers").click({ button: "right" });
  await page.getByRole("menuitem", { name: "设计表" }).click();
  await page.getByRole("textbox", { name: "name：字段名" }).fill("full_name");
  await page.getByRole("button", { name: "新增字段" }).click();
  await page.keyboard.type("age");
  await expect(page.getByRole("button", { name: /保存/ })).toContainText("2");
}

test("table designer", async ({ page }) => {
  await designCustomers(page);
  await snapshot(page, "table-designer");
});

test("table designer: save preview", async ({ page }) => {
  await designCustomers(page);
  await page.keyboard.press("Meta+s");
  const dialog = page.getByRole("dialog", { name: "保存表结构" });
  await expect(dialog.getByRole("list", { name: "将要执行的语句" })).toBeVisible();
  await snapshot(page, "table-designer-save");
});

/** 在 customers 上右键,选导入导出的菜单项。 */
async function customersMenu(page: Page, item: string) {
  await openWelcome(page);
  await expandShop(page);
  await row(page, "customers").click({ button: "right" });
  await page.getByRole("menuitem", { name: item }).click();
}

test("export: options", async ({ page }) => {
  await customersMenu(page, "导出数据…");
  const dialog = page.getByRole("dialog", { name: "导出数据" });
  await dialog.getByRole("radio", { name: /Excel/ }).click();
  await expect(dialog.getByRole("radio", { name: /Excel/ })).toHaveAttribute("aria-checked", "true");
  await snapshot(page, "export-options");
});

test("export: done", async ({ page }) => {
  await customersMenu(page, "导出数据…");
  await page.getByRole("dialog", { name: "导出数据" }).getByRole("button", { name: "导出…" }).click();
  await expect(page.getByRole("dialog", { name: "导出完成" })).toBeVisible();
  await snapshot(page, "export-done");
});

/** 导入向导:选好文件,停在第二步。 */
async function importMapping(page: Page) {
  await customersMenu(page, "导入数据…");
  const dialog = page.getByRole("dialog", { name: "导入数据" });
  await dialog.getByRole("button", { name: "选择文件…" }).click();
  await expect(dialog.getByRole("table", { name: "文件预览" })).toBeVisible();
  return dialog;
}

test("import: file", async ({ page }) => {
  await importMapping(page);
  await snapshot(page, "import-file");
});

test("import: mapping", async ({ page }) => {
  const dialog = await importMapping(page);
  await dialog.getByRole("button", { name: "下一步" }).click();
  await expect(dialog.getByRole("table", { name: "列映射" })).toBeVisible();
  await snapshot(page, "import-mapping");
});

test("import: result with failures", async ({ page }) => {
  const dialog = await importMapping(page);
  await dialog.getByRole("button", { name: "下一步" }).click();
  await dialog.getByRole("button", { name: "下一步" }).click();
  await dialog.getByRole("radio", { name: /跳过并记录/ }).click();
  await dialog.getByRole("button", { name: "开始导入" }).click();
  await expect(dialog.getByRole("table", { name: "失败的行" })).toBeVisible();
  await snapshot(page, "import-result");
});
