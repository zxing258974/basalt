// 平台差异:macOS 用 ⌘、钥匙串、访达;Windows 用 Ctrl、凭据管理器、资源管理器。
// 按 WebView 的 User-Agent 判断:macOS 的 WKWebView 带「Macintosh」,Windows 的 WebView2 带「Windows」。

export type Platform = "mac" | "windows";

let override: Platform | null = null;

/** 测试里固定平台,不随跑测试的机器变。传 null 恢复自动判断。 */
export function setPlatformForTesting(platform: Platform | null): void {
  override = platform;
}

export function platform(): Platform {
  if (override) return override;
  const agent = typeof navigator === "undefined" ? "" : navigator.userAgent;
  return /Windows/i.test(agent) ? "windows" : "mac";
}

/** 主修饰键按下了:macOS 是 ⌘,Windows 是 Ctrl。两个都按着(⌃⌘)不算。 */
export function hasPrimaryModifier(event: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return platform() === "mac" ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

const MAC_KEYS: Record<string, string> = { Enter: "↩", Backspace: "⌫" };

/** 快捷键的写法:shortcut("S") → ⌘S / Ctrl+S;shortcut("Enter", { shift: true }) → ⇧⌘↩ / Ctrl+Shift+Enter。 */
/** 单个按键的名字:macOS 用符号(↩ ⌫),Windows 用键帽上的字。 */
export function keyLabel(key: string): string {
  return platform() === "mac" ? (MAC_KEYS[key] ?? key) : key;
}

export function shortcut(key: string, options: { shift?: boolean } = {}): string {
  if (platform() === "mac") return `${options.shift ? "⇧" : ""}⌘${MAC_KEYS[key] ?? key}`;
  return ["Ctrl", options.shift ? "Shift" : null, key].filter(Boolean).join("+");
}

/** 存密码的地方叫什么。 */
export function secretStoreName(): string {
  return platform() === "mac" ? "钥匙串" : "Windows 凭据管理器";
}

/** 系统的文件管理器叫什么(「在访达中显示」)。 */
/** 路径里的文件名。Windows 路径用反斜杠,macOS 用正斜杠,两种都认。 */
export function fileName(path: string): string {
  return path.split(/[\\/]/).at(-1) || path;
}

export function fileManagerName(): string {
  return platform() === "mac" ? "访达" : "资源管理器";
}
