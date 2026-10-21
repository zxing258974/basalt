// Windows 的 WebView2 自带浏览器快捷键和右键菜单:F5 / Ctrl+R 会把整个界面刷新掉(没保存的修改、
// 正在跑的查询都会丢),Ctrl+P 弹打印,空白处右键有「刷新」「另存为」。
// 快捷键在启动时已由 Rust 端关掉(src-tauri/src/webview_windows.rs),这里的按键拦截是兜底(太老的
// WebView2 运行时没有那个开关)。右键菜单只能在这里拦:整个关掉的话,输入框里的剪切、粘贴菜单也没了。
// macOS 的 WKWebView 没有这些,不用装。

/** 一律拦下的:刷新和打印。 */
function isReloadOrPrint(event: KeyboardEvent): boolean {
  if (event.key === "F5" || event.key === "BrowserRefresh") return true;
  if (!event.ctrlKey || event.altKey) return false;
  const key = event.key.toLowerCase();
  return key === "r" || key === "p";
}

/** 能打字的输入框类型;复选框、单选框这类没有剪切粘贴,右键只会弹出浏览器菜单。 */
const TEXT_INPUTS = new Set(["text", "search", "password", "email", "url", "tel", "number"]);

/** 输入框里留着系统的剪切、复制、粘贴菜单。 */
function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  const field = target.closest("input, textarea, [contenteditable=''], [contenteditable='true']");
  if (!field) return false;
  return !(field instanceof HTMLInputElement) || TEXT_INPUTS.has(field.type);
}

/** 装上拦截,返回卸载函数。 */
export function installWebviewGuards(target: Window): () => void {
  // 捕获阶段:哪个组件阻止了冒泡也拦得住。
  const onKeyDownCapture = (event: KeyboardEvent) => {
    if (isReloadOrPrint(event)) event.preventDefault();
  };
  // 冒泡阶段:SQL 编辑器自己处理 Ctrl+F(查找替换);没人处理的才拦,不弹 WebView2 的页内查找。
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || !event.ctrlKey || event.altKey) return;
    if (event.key.toLowerCase() === "f") event.preventDefault();
  };
  // 自己的右键菜单都会 preventDefault;没人处理的空白处不弹 WebView2 的菜单。
  const onContextMenu = (event: MouseEvent) => {
    if (!isEditable(event.target)) event.preventDefault();
  };
  target.addEventListener("keydown", onKeyDownCapture, true);
  target.addEventListener("keydown", onKeyDown);
  target.addEventListener("contextmenu", onContextMenu);
  return () => {
    target.removeEventListener("keydown", onKeyDownCapture, true);
    target.removeEventListener("keydown", onKeyDown);
    target.removeEventListener("contextmenu", onContextMenu);
  };
}
