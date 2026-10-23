//! 关窗口和退出。
//!
//! macOS 上点主窗口的关闭按钮只把窗口收起来,应用留在程序坞,点程序坞图标再叫回来(见 [`on_run_event`]);
//! ⌘Q、程序坞右键「退出」、关机或注销才真正退出。Windows 上只有这一个窗口,点关闭按钮、Alt+F4 就是退出。
//!
//! 退出前确认:前端有未提交的表格修改、没保存的查询就先问一句。
//! 前端一有变化就用 [`set_unsaved_work`] 报告有没有这样的东西。没有就照常退出,不等前端
//! (前端卡住、没加载出来也退得掉);有就拦下来,先把主窗口叫回来(收着的话确认框弹在看不见的窗口里,
//! 看起来就像退不出去),再发 [`QUIT_REQUESTED`] 给前端,前端弹确认框,用户确认放弃后调 [`quit_app`] 真正退出。
//!
//! 程序坞的「退出」和关机走的是系统的 `terminate:`,不经过菜单,tao 也没接 `applicationShouldTerminate:`:
//! 启动时给 tao 的应用代理补上这个方法(见 [`install_terminate_guard`]),拦下来时回答「先不退」。
//! 关机、注销会因此停下(系统提示 Basalt 中断了),用户在确认框里放弃修改后应用退出,再关一次机即可。

use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, State, Window, WindowEvent};

/// 拦下退出时发给前端的事件,和 src/lib/api.ts 一致。
pub const QUIT_REQUESTED: &str = "quit-requested";
/// 自己的「Quit」菜单项。
#[cfg(target_os = "macos")]
const QUIT_MENU_ID: &str = "basalt-quit";
/// tauri.conf.json 里主窗口的 label。
const MAIN_WINDOW: &str = "main";

/// 前端最近一次报告的「有没有未保存的东西」,以及用户是不是已经确认要退出了。
#[derive(Debug, Default)]
pub struct QuitGuard {
    unsaved: AtomicBool,
    confirmed: AtomicBool,
}

impl QuitGuard {
    pub fn set_unsaved(&self, unsaved: bool) {
        self.unsaved.store(unsaved, Ordering::SeqCst);
    }

    pub fn has_unsaved(&self) -> bool {
        self.unsaved.load(Ordering::SeqCst)
    }

    /// 确认过了:之后的退出(包括 `app.exit` 自己走到的 `terminate:`)不再拦。
    fn confirm(&self) {
        self.confirmed.store(true, Ordering::SeqCst);
    }

    fn confirmed(&self) -> bool {
        self.confirmed.load(Ordering::SeqCst)
    }
}

/// 把主窗口叫回来,放到最前面:点了关闭按钮收起来的、缩进程序坞的、全屏时跟着应用一起藏起来的都算。
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    if let Err(error) = app.show() {
        log::warn!("没能取消隐藏应用:{error}");
    }
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    for (step, result) in [
        ("显示", window.show()),
        ("还原", window.unminimize()),
        ("激活", window.set_focus()),
    ] {
        if let Err(error) = result {
            log::warn!("没能{step}主窗口:{error}");
        }
    }
}

/// 点主窗口的关闭按钮时把它收起来。全屏时 `orderOut:` 会在全屏空间里留下一块黑屏,
/// 改成隐藏整个应用:系统自己退出那个空间,点程序坞图标再回到全屏。
#[cfg(target_os = "macos")]
fn hide_main_window<R: Runtime>(window: &Window<R>) {
    let result = if window.is_fullscreen().unwrap_or(false) {
        window.app_handle().hide()
    } else {
        window.hide()
    };
    if let Err(error) = result {
        log::error!("没能收起主窗口:{error}");
    }
}

/// 有未保存的东西就交给前端确认,返回 true 表示这次退出要拦下。
fn hand_to_frontend<R: Runtime>(app: &AppHandle<R>) -> bool {
    if !app.state::<QuitGuard>().has_unsaved() {
        return false;
    }
    show_main_window(app);
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
#[cfg(target_os = "macos")]
pub fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == QUIT_MENU_ID && !hand_to_frontend(app) {
        app.exit(0);
    }
}

/// 点主窗口的关闭按钮。macOS 上只收起窗口,什么都不会丢,不用问;
/// 别的平台只有这一个窗口,关了应用也就退出了,有未保存的东西先问。
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() != MAIN_WINDOW {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        api.prevent_close();
        hide_main_window(window);
    }
    #[cfg(not(target_os = "macos"))]
    if hand_to_frontend(window.app_handle()) {
        api.prevent_close();
    }
}

/// 应用级事件:点程序坞图标(macOS 的 Reopen)时把收起来的主窗口叫回来。
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent) {
    // 窗口开着时也叫一次:全屏时收起的是整个应用,系统取消隐藏的时机说不准,show 一下没有坏处。
    #[cfg(target_os = "macos")]
    if let RunEvent::Reopen { .. } = event {
        show_main_window(app);
    }
}

#[tauri::command]
pub fn set_unsaved_work(guard: State<'_, QuitGuard>, unsaved: bool) {
    guard.set_unsaved(unsaved);
}

/// 前端确认过(或本来就没有未保存的东西),直接退出。
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.state::<QuitGuard>().confirm();
    app.exit(0);
}

/// 系统要退出(`terminate:`:程序坞的「退出」、关机、注销)时:返回 true 表示现在就退,
/// false 表示先不退、已经交给前端确认。
pub fn allow_terminate<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<QuitGuard>().confirmed() || !hand_to_frontend(app)
}

/// 给 tao 的应用代理补上 `applicationShouldTerminate:`。setup 里调一次。
#[cfg(target_os = "macos")]
pub fn install_terminate_guard(app: &AppHandle) {
    terminate::install(app);
}

#[cfg(target_os = "macos")]
mod terminate {
    use std::sync::OnceLock;

    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{class, msg_send, sel};
    use tauri::AppHandle;

    /// NSApplicationTerminateReply。
    const TERMINATE_CANCEL: usize = 0;
    const TERMINATE_NOW: usize = 1;

    static APP: OnceLock<AppHandle> = OnceLock::new();

    extern "C-unwind" fn should_terminate(
        _this: &AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) -> usize {
        match APP.get() {
            Some(app) if !super::allow_terminate(app) => TERMINATE_CANCEL,
            _ => TERMINATE_NOW,
        }
    }

    pub(super) fn install(app: &AppHandle) {
        if APP.set(app.clone()).is_err() {
            return;
        }
        let selector = sel!(applicationShouldTerminate:);
        // SAFETY:都在主线程(setup)里调;取到的是 tao 建的应用代理,类是它运行时注册的,
        // 补的方法签名和 AppKit 声明的一致:- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)。
        unsafe {
            let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let delegate: *mut AnyObject = msg_send![ns_app, delegate];
            let Some(delegate) = delegate.as_ref() else {
                log::warn!("没有应用代理,从程序坞退出、关机时不会提醒未保存的修改");
                return;
            };
            let class = delegate.class();
            if class.instance_method(selector).is_some() {
                log::warn!("应用代理已经有 applicationShouldTerminate:,不覆盖");
                return;
            }
            let imp: Imp = std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
                Imp,
            >(should_terminate);
            let added = objc2::ffi::class_addMethod(
                std::ptr::from_ref::<AnyClass>(class).cast_mut(),
                selector,
                imp,
                c"Q@:@".as_ptr(),
            );
            if !added.as_bool() {
                log::warn!("没能给应用代理补上 applicationShouldTerminate:");
                return;
            }
            // AppKit 可能在设置代理时就记下了它有哪些方法:重新设一次,让它看到新补的。
            let _: () = msg_send![ns_app, setDelegate: delegate];
        }
    }
}

/// Tauri 的默认菜单,只把 macOS 应用菜单里的「Quit」换成自己的:
/// 系统那个直接走 `terminate:`,不经过 Tauri 的任何事件,拦不住。
#[cfg(target_os = "macos")]
pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    menu.remove_at(0)?;
    menu.insert(&macos::app_submenu(app)?, 0)?;
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

    #[cfg(target_os = "macos")]
    use tauri::menu::{MenuEvent, MenuId};
    use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
    use tauri::{App, Listener, Manager, WebviewUrl, WebviewWindowBuilder};

    use super::*;

    /// 和 lib.rs 一样接上退出前确认的 mock 应用,开着主窗口;返回值里的计数是前端收到的退出请求。
    /// mock 运行时的 `app.exit` 是 `unimplemented!()`:该拦下却直接退出时测试会 panic。
    fn app(unsaved: bool) -> (App<MockRuntime>, Arc<AtomicUsize>) {
        let builder = mock_builder();
        #[cfg(target_os = "macos")]
        let builder = builder.on_menu_event(on_menu_event);
        let app = builder
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

    #[cfg(target_os = "macos")]
    fn menu_event(id: &str) -> MenuEvent {
        MenuEvent {
            id: MenuId::new(id),
        }
    }

    #[test]
    fn the_system_quit_goes_ahead_when_nothing_is_unsaved() {
        let (app, requests) = app(false);
        assert!(allow_terminate(app.handle()));
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_system_quit_waits_for_the_frontend_when_work_is_unsaved() {
        let (app, requests) = app(true);
        assert!(!allow_terminate(app.handle()));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn once_confirmed_the_system_quit_is_not_stopped_again() {
        let (app, requests) = app(true);
        app.state::<QuitGuard>().confirm();
        assert!(allow_terminate(app.handle()));
        assert_eq!(requests.load(Ordering::SeqCst), 0);
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

    /// macOS:关主窗口只是收起来,应用不退出,所以有没有未保存的东西都不问。
    /// (mock 窗口的显示状态不会变,收起来本身只能在真窗口里看。)
    #[cfg(target_os = "macos")]
    #[test]
    fn closing_the_main_window_only_hides_it() {
        for unsaved in [false, true] {
            let (app, requests) = app(unsaved);
            assert_eq!(press_close(&app, MAIN_WINDOW).try_recv(), Ok(true));
            assert_eq!(requests.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn bringing_the_main_window_back_works_with_or_without_it() {
        let (app, requests) = app(true);
        show_main_window(app.handle());
        app.get_webview_window(MAIN_WINDOW)
            .expect("main window")
            .destroy()
            .expect("destroy");
        show_main_window(app.handle());
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn closing_the_main_window_goes_ahead_when_nothing_is_unsaved() {
        let (app, requests) = app(false);
        assert!(press_close(&app, MAIN_WINDOW).try_recv().is_err());
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[cfg(not(target_os = "macos"))]
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

    #[cfg(target_os = "macos")]
    #[test]
    fn quit_menu_asks_the_frontend_instead_of_quitting_when_work_is_unsaved() {
        let (app, requests) = app(true);
        on_menu_event(app.handle(), menu_event(QUIT_MENU_ID));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn other_menu_items_are_left_alone() {
        let (app, requests) = app(false);
        on_menu_event(app.handle(), menu_event("about"));
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }
}
