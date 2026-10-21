// 平台差异:macOS 用 ⌘ 和钥匙串,Windows 用 Ctrl 和凭据管理器。其余测试固定按 macOS 跑(见 test/setup.ts)。
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  fileManagerName,
  fileName,
  hasPrimaryModifier,
  keyLabel,
  platform,
  secretStoreName,
  setPlatformForTesting,
  shortcut,
} from "./platform";

const key = (metaKey: boolean, ctrlKey: boolean) => ({ metaKey, ctrlKey });

describe("platform", () => {
  afterEach(() => {
    setPlatformForTesting("mac");
    vi.unstubAllGlobals();
  });

  it("tells macOS and Windows apart from the WebView's user agent", () => {
    setPlatformForTesting(null);
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15" });
    expect(platform()).toBe("mac");
    vi.stubGlobal("navigator", {
      userAgent:
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Edg/130.0",
    });
    expect(platform()).toBe("windows");
  });

  it("uses ⌘ and the keychain on macOS", () => {
    setPlatformForTesting("mac");
    expect([hasPrimaryModifier(key(true, false)), hasPrimaryModifier(key(false, true))]).toEqual([true, false]);
    expect(hasPrimaryModifier(key(true, true))).toBe(false);
    expect([shortcut("S"), shortcut("Enter", { shift: true }), shortcut("Backspace")]).toEqual(["⌘S", "⇧⌘↩", "⌘⌫"]);
    expect([secretStoreName(), fileManagerName()]).toEqual(["钥匙串", "访达"]);
    expect([keyLabel("Enter"), keyLabel("Esc")]).toEqual(["↩", "Esc"]);
  });

  it("uses Ctrl and the Credential Manager on Windows", () => {
    setPlatformForTesting("windows");
    expect([hasPrimaryModifier(key(false, true)), hasPrimaryModifier(key(true, false))]).toEqual([true, false]);
    expect([shortcut("S"), shortcut("Enter", { shift: true }), shortcut(".")]).toEqual([
      "Ctrl+S",
      "Ctrl+Shift+Enter",
      "Ctrl+.",
    ]);
    expect([secretStoreName(), fileManagerName()]).toEqual(["Windows 凭据管理器", "资源管理器"]);
    expect(keyLabel("Enter")).toBe("Enter");
  });

  it("finds the file name in macOS and Windows paths", () => {
    expect(fileName("/Users/me/a.csv")).toBe("a.csv");
    expect(fileName("C:\\Users\\me\\订单.xlsx")).toBe("订单.xlsx");
    expect(fileName("D:/data/shop.db")).toBe("shop.db");
    expect([fileName("a.db"), fileName("")]).toEqual(["a.db", ""]);
  });
});
