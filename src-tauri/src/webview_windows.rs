//! Windows:启动时关掉 WebView2 自带的浏览器快捷键(F5 / Ctrl+R 刷新整个界面、Ctrl+P 打印、
//! Ctrl+F / F3 / Ctrl+G 页内查找、Alt+← 后退……)。Tauri 没把这个开关放进配置,只能拿到 WebView2 自己设。
//! 复制、粘贴、撤销这些编辑键不受影响。前端 src/lib/webviewGuards.ts 再兜一层底。

use tauri::webview::PlatformWebview;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
use windows_core::Interface;

pub fn disable_browser_shortcuts(window: &tauri::WebviewWindow) {
    let result = window.with_webview(|webview| {
        if let Err(error) = turn_off(&webview) {
            log::warn!("没能关掉 WebView2 的浏览器快捷键:{error}");
        }
    });
    if let Err(error) = result {
        log::warn!("拿不到 WebView2:{error}");
    }
}

fn turn_off(webview: &PlatformWebview) -> windows_core::Result<()> {
    // SAFETY: with_webview 在 WebView2 所在的线程上调用;COM 接口的引用计数由 webview2-com 管。
    // 太老的 WebView2 运行时没有 Settings3,cast 会返回错误,不会崩。
    unsafe {
        let settings = webview.controller().CoreWebView2()?.Settings()?;
        settings
            .cast::<ICoreWebView2Settings3>()?
            .SetAreBrowserAcceleratorKeysEnabled(false)
    }
}
