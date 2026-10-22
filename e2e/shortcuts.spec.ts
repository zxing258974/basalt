// 编辑器快捷键在真实 WebKit 里按:jsdom 的键盘事件不全(按着 Shift 的字母键没有 keyCode),这类只能在这里验。
import { expect, test } from "@playwright/test";
import { openWelcome, row } from "./helpers";

test.skip(({ colorScheme, viewport }) => colorScheme === "dark" || viewport?.width !== 1280, "只验按键，一种窗口就够");

test("⇧⌘F formats the SQL in the editor", async ({ page }) => {
  await openWelcome(page);
  await row(page, "开发").dblclick();
  await row(page, "本地 MySQL").dblclick();
  await row(page, "shop").click({ button: "right" });
  await page.getByRole("menuitem", { name: /新建查询/ }).click();
  const editor = page.getByRole("textbox", { name: "查询 1 的 SQL" });
  await editor.click();
  await page.keyboard.type("select id from customers where vip = 1");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Meta+Shift+F");
  // 每行是一个元素,文字连在一起:换了行、缩进了(MySQL 的关键字照原样小写)。
  await expect(editor).toHaveText(/^select\s+id\s*from\s+customers\s*where\s+vip = 1$/);
  await expect(editor.locator(".cm-line")).toHaveCount(6);
});
