//! 退出前确认:⌘Q 或点主窗口的关闭按钮时,前端有未提交的表格修改、没保存的查询就先问一句。
//!
//! 前端一有变化就用 [`set_unsaved_work`] 报告有没有这样的东西。没有就照常退出,不等前端
//! (前端卡住、没加载出来也退得掉);有就拦下来,发 [`QUIT_REQUESTED`] 给前端,
//! 前端弹确认框,用户确认放弃后调 [`quit_app`] 真正退出。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, State, Window, WindowEvent};

/// 拦下退出时发给前端的事件,和 src/lib/api.ts 一致。
pub const QUIT_REQUESTED: &str = "quit-requested";
/// 自己的「Quit」菜单项。
const QUIT_MENU_ID: &str = "basalt-quit";
/// tauri.conf.json 里主窗口的 label。
const MAIN_WINDOW: &str = "main";

/// 前端最近一次报告的「有没有未保存的东西」。
#[derive(Debug, Default)]
pub struct QuitGuard {
    unsaved: AtomicBool,
}

impl QuitGuard {
    pub fn set_unsaved(&self, unsaved: bool) {
        self.unsaved.store(unsaved, Ordering::SeqCst);
    }

    pub fn has_unsaved(&self) -> bool {
        self.unsaved.load(Ordering::SeqCst)
    }
}

/// 有未保存的东西就交给前端确认,返回 true 表示这次退出要拦下。
fn hand_to_frontend<R: Runtime>(app: &AppHandle<R>) -> bool {
    if !app.state::<QuitGuard>().has_unsaved() {
        return false;
    }
    match app.emit_to(MAIN_WINDOW, QUIT_REQUESTED, ()) {
        Ok(()) => true,
        Err(error) => {
            // 前端收不到就没人能放行,拦下来只会退不出去。
            log::error!("没能让前端确认退出,直接退出:{error}");
            false
        }
    }
}

/// 菜单里的「Quit」(⌘Q)。
pub fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == QUIT_MENU_ID && !hand_to_frontend(app) {
        app.exit(0);
    }
}

/// 点主窗口的关闭按钮。只有这一个窗口,关了应用也就退出了。
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event
        && window.label() == MAIN_WINDOW
        && hand_to_frontend(window.app_handle())
    {
        api.prevent_close();
    }
}

#[tauri::command]
pub fn set_unsaved_work(guard: State<'_, QuitGuard>, unsaved: bool) {
    guard.set_unsaved(unsaved);
}

/// 前端确认过(或本来就没有未保存的东西),直接退出。
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Tauri 的默认菜单,只把 macOS 应用菜单里的「Quit」换成自己的:
/// 系统那个直接走 `terminate:`,不经过 Tauri 的任何事件,拦不住。
pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    #[cfg(target_os = "macos")]
    {
        menu.remove_at(0)?;
        menu.insert(&macos::app_submenu(app)?, 0)?;
    }
    Ok(menu)
}

#[cfg(target_os = "macos")]
mod macos {
    use tauri::menu::{AboutMetadata, MenuItem, PredefinedMenuItem, Submenu};
    use tauri::{AppHandle, Runtime};

    use super::QUIT_MENU_ID;

    /// 和 `Menu::default` 里的应用菜单一样,只有最后一项换掉。
    pub(super) fn app_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
        let name = app.package_info().name.clone();
        let bundle = &app.config().bundle;
        let about = AboutMetadata {
            name: Some(name.clone()),
            version: Some(app.package_info().version.to_string()),
            copyright: bundle.copyright.clone(),
            authors: bundle.publisher.clone().map(|publisher| vec![publisher]),
            ..Default::default()
        };
        let quit = MenuItem::with_id(
            app,
            QUIT_MENU_ID,
            format!("Quit {name}"),
            true,
            Some("CmdOrCtrl+Q"),
        )?;
        Submenu::with_items(
            app,
            name,
            true,
            &[
                &PredefinedMenuItem::about(app, None, Some(about))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &quit,
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};

    use tauri::menu::{MenuEvent, MenuId};
    use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
    use tauri::{App, Listener, Manager, WebviewUrl, WebviewWindowBuilder};

    use super::*;

    /// 和 lib.rs 一样接上退出前确认的 mock 应用,开着主窗口;返回值里的计数是前端收到的退出请求。
    /// mock 运行时的 `app.exit` 是 `unimplemented!()`:该拦下却直接退出时测试会 panic。
    fn app(unsaved: bool) -> (App<MockRuntime>, Arc<AtomicUsize>) {
        let app = mock_builder()
            .on_menu_event(on_menu_event)
            .on_window_event(on_window_event)
            .manage(QuitGuard::default())
            .build(mock_context(noop_assets()))
            .expect("mock app");
        WebviewWindowBuilder::new(&app, MAIN_WINDOW, WebviewUrl::default())
            .build()
            .expect("main window");
        app.state::<QuitGuard>().set_unsaved(unsaved);
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        app.listen_any(QUIT_REQUESTED, move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        (app, requests)
    }

    /// 让窗口收到一次点关闭按钮;返回的通道里有 `true` 表示被拦下了。
    /// mock 运行时不把窗口事件转给 `on_window_event`,只能直接调。
    fn press_close(app: &App<MockRuntime>, label: &str) -> Receiver<bool> {
        let (signal_tx, signal_rx) = channel();
        let event =
            WindowEvent::from(tauri_runtime::window::WindowEvent::CloseRequested { signal_tx });
        let webview = app.get_webview_window(label).expect("window");
        on_window_event(&webview.as_ref().window(), &event);
        signal_rx
    }

    fn menu_event(id: &str) -> MenuEvent {
        MenuEvent {
            id: MenuId::new(id),
        }
    }

    #[test]
    fn nothing_is_unsaved_until_the_frontend_says_so() {
        assert!(!QuitGuard::default().has_unsaved());
    }

    #[test]
    fn follows_the_latest_report() {
        let guard = QuitGuard::default();
        guard.set_unsaved(true);
        assert!(guard.has_unsaved());
        guard.set_unsaved(false);
        assert!(!guard.has_unsaved());
    }

    #[test]
    fn closing_the_main_window_goes_ahead_when_nothing_is_unsaved() {
        let (app, requests) = app(false);
        assert!(press_close(&app, MAIN_WINDOW).try_recv().is_err());
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn closing_the_main_window_with_unsaved_work_waits_for_the_frontend() {
        let (app, requests) = app(true);
        assert_eq!(press_close(&app, MAIN_WINDOW).try_recv(), Ok(true));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn other_windows_close_without_asking() {
        let (app, requests) = app(true);
        WebviewWindowBuilder::new(&app, "other", WebviewUrl::default())
            .build()
            .expect("other window");
        assert!(press_close(&app, "other").try_recv().is_err());
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn quit_menu_asks_the_frontend_instead_of_quitting_when_work_is_unsaved() {
        let (app, requests) = app(true);
        on_menu_event(app.handle(), menu_event(QUIT_MENU_ID));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn other_menu_items_are_left_alone() {
        let (app, requests) = app(false);
        on_menu_event(app.handle(), menu_event("about"));
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }
}
