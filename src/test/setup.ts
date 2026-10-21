import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { setBackendForTesting } from "../lib/api";
import { setPlatformForTesting } from "../lib/platform";

// 快捷键和文案按 macOS 写的测试不随跑测试的机器变;Windows 的写法在 platform.test.ts 里单独测。
setPlatformForTesting("mac");

afterEach(() => {
  cleanup();
  setBackendForTesting(null);
});
