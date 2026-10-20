import { defineConfig, type Project } from "@playwright/test";

// 截图测试:浏览器里跑 `pnpm dev`,前端自动走 mock 后端,数据固定。
// 只用 WebKit:Tauri 在 macOS 上用的就是它,Chromium 的排版和它有差异(出过 grid 被 input 撑开的问题)。

/** 单独的端口,不和 Tauri 固定的 1420(可能正开着 `pnpm tauri dev`)抢。 */
const PORT = 1430;
/** 桌面窗口的最小宽度、默认宽度和一档更宽的;高度取默认窗口高度。 */
const WIDTHS = [960, 1280, 1440];
const WINDOW_HEIGHT = 820;
const THEMES = ["light", "dark"] as const;

const projects: Project[] = THEMES.flatMap((theme) =>
  WIDTHS.map((width) => ({
    name: `webkit-${width}-${theme}`,
    use: {
      browserName: "webkit" as const,
      viewport: { width, height: WINDOW_HEIGHT },
      colorScheme: theme,
    },
  })),
);

export default defineConfig({
  testDir: "e2e",
  snapshotPathTemplate: "{testDir}/__screenshots__/{arg}-{projectName}{ext}",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    locale: "zh-CN",
    reducedMotion: "reduce",
    trace: "retain-on-failure",
  },
  expect: {
    toHaveScreenshot: { animations: "disabled", caret: "hide" },
  },
  projects,
  webServer: {
    command: `pnpm dev --port ${PORT}`,
    url: `http://localhost:${PORT}`,
    // 端口上已有的服务可能是别的检出目录起的,截到的不是这份代码;宁可报端口被占。
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
