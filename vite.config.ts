/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { configDefaults } from "vitest/config";

const host = process.env.TAURI_DEV_HOST;

// Tauri 固定使用 1420 端口;不清屏以免吞掉 Rust 的编译输出。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
  build: {
    target: "safari16",
  },
  test: {
    environment: "jsdom",
    // .claude/worktrees 下是其他会话的检出,各有各的 node_modules,不能混进这里跑。
    exclude: [...configDefaults.exclude, ".claude/**"],
    setupFiles: ["./src/test/setup.ts"],
    css: false,
  },
});
