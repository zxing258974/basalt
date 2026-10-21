// 右侧面板拖宽后的真实布局(WebKit):往左拖变宽,给旁边留够 320px,窗口缩小后也不把页面撑出窗口。
import { expect, test, type Locator, type Page } from "@playwright/test";
import { askAssistant, expectFitsWindow, inspectCustomers } from "./helpers";

/** 和 src/hooks/usePanelWidth.ts 的 REST_MIN 一致。 */
const REST_MIN = 320;

test.skip(({ colorScheme }) => colorScheme === "dark", "只量布局,和深浅主题无关");

const width = async (locator: Locator) => (await locator.boundingBox())!.width;

/** 分隔条本身 0 宽(Playwright 认为它不可见,不能直接点),把手是两边各伸出的 3px。 */
async function handle(separator: Locator) {
  const box = (await separator.boundingBox())!;
  return { x: box.x, y: box.y + box.height / 2 };
}

async function drag(page: Page, separator: Locator, dx: number) {
  const { x, y } = await handle(separator);
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y, { steps: 6 });
  await page.mouse.up();
}

test("inspector widens from its left edge and leaves room for the workspace", async ({ page }) => {
  const panel = await inspectCustomers(page);
  const workspace = page.locator("main.workspace");
  const separator = page.getByRole("separator", { name: "调整信息面板宽度" });
  const before = await width(panel);
  expect(before).toBeCloseTo(280, 0);
  const room = Math.max(0, (await width(workspace)) - REST_MIN);

  await drag(page, separator, -150);
  expect(await width(panel)).toBeCloseTo(before + Math.min(150, room), 0);
  expect(await width(workspace)).toBeGreaterThanOrEqual(REST_MIN - 1);
  await expectFitsWindow(page);

  const { x, y } = await handle(separator);
  await page.mouse.dblclick(x, y);
  expect(await width(panel)).toBeCloseTo(280, 0);
});

test("inspector keeps the workspace usable when the window shrinks", async ({ page }) => {
  const panel = await inspectCustomers(page);
  const workspace = page.locator("main.workspace");
  const separator = page.getByRole("separator", { name: "调整信息面板宽度" });
  const room = Math.max(0, (await width(workspace)) - REST_MIN);
  await separator.focus();
  await page.keyboard.press("End");
  expect(await width(panel)).toBeCloseTo(Math.min(600, 280 + room), 0);
  expect(await width(workspace)).toBeGreaterThanOrEqual(REST_MIN - 1);

  await page.setViewportSize({ width: 960, height: 820 });
  expect(await width(workspace)).toBeGreaterThanOrEqual(REST_MIN - 1);
  await expectFitsWindow(page);
});

test("assistant widens from its left edge and keeps room for the editor", async ({ page }) => {
  const panel = await askAssistant(page);
  const main = page.locator(".query-main");
  const separator = page.getByRole("separator", { name: "调整 SQL 助手宽度" });
  const before = await width(panel);
  expect(before).toBeCloseTo(340, 0);
  const room = Math.max(0, (await width(main)) - REST_MIN);

  await drag(page, separator, -200);
  expect(await width(panel)).toBeCloseTo(before + Math.min(200, room), 0);
  expect(await width(main)).toBeGreaterThanOrEqual(REST_MIN - 1);
  await expectFitsWindow(page);

  await drag(page, separator, 400);
  expect(await width(panel)).toBeCloseTo(280, 0);
});
