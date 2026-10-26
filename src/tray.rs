//! System tray icon. Closing the window hides it there while tracking and OSC keep running;
//! "Quit" in the tray menu closes the app. Windows only: other platforms have no tray.

#[cfg(windows)]
pub use imp::Tray;

#[cfg(not(windows))]
pub use stub::Tray;

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
    };

    use crate::i18n::{Language, tr};

    pub struct Tray {
        // Dropping the icon removes it from the tray.
        _icon: TrayIcon,
        open: MenuItem,
        quit: MenuItem,
        language: Language,
        hwnd: isize,
        quitting: Arc<AtomicBool>,
    }

    fn handle(hwnd: isize) -> HWND {
        HWND(hwnd as *mut c_void)
    }

    /// Brings the window back. Done with Win32 calls instead of viewport commands because a
    /// hidden window may not run the UI loop to process them.
    fn show(hwnd: isize, ctx: &egui::Context) {
        unsafe {
            let _ = ShowWindow(handle(hwnd), SW_SHOW);
            let _ = ShowWindow(handle(hwnd), SW_RESTORE);
            let _ = SetForegroundWindow(handle(hwnd));
        }
        ctx.request_repaint();
    }

    impl Tray {
        pub fn new(cc: &eframe::CreationContext<'_>, language: Language) -> Option<Self> {
            let hwnd = match cc.window_handle().ok()?.as_raw() {
                RawWindowHandle::Win32(window) => window.hwnd.get(),
                _ => return None,
            };
            let image = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok()?;
            let icon = Icon::from_rgba(image.rgba, image.width, image.height).ok()?;
            let open = MenuItem::new(tr(language, "tray_open"), true, None);
            let quit = MenuItem::new(tr(language, "tray_quit"), true, None);
            let menu = Menu::with_items(&[&open, &PredefinedMenuItem::separator(), &quit]).ok()?;
            let tray = TrayIconBuilder::new()
                .with_icon(icon)
                .with_tooltip("VRC Openface")
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|e| log::warn!("tray icon: {e}"))
                .ok()?;

            let ctx = cc.egui_ctx.clone();
            let quitting = Arc::new(AtomicBool::new(false));
            let tray_ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                    event
                {
                    show(hwnd, &tray_ctx);
                }
            }));
            let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
            let quit_flag = quitting.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if event.id == open_id {
                    show(hwnd, &ctx);
                } else if event.id == quit_id {
                    // The window is shown first so its loop is running to handle the close.
                    quit_flag.store(true, Ordering::Relaxed);
                    show(hwnd, &ctx);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }));
            Some(Self { _icon: tray, open, quit, language, hwnd, quitting })
        }

        /// Keeps the menu text in the interface language.
        pub fn set_language(&mut self, language: Language) {
            if self.language != language {
                self.language = language;
                self.open.set_text(tr(language, "tray_open"));
                self.quit.set_text(tr(language, "tray_quit"));
            }
        }

        /// True once "Quit" was chosen in the tray menu.
        pub fn quitting(&self) -> bool {
            self.quitting.load(Ordering::Relaxed)
        }

        pub fn hide_window(&self) {
            unsafe {
                let _ = ShowWindow(handle(self.hwnd), SW_HIDE);
            }
        }

        pub fn window_visible(&self) -> bool {
            unsafe { IsWindowVisible(handle(self.hwnd)).as_bool() }
        }
    }
}

#[cfg(not(windows))]
mod stub {
    use crate::i18n::Language;

    /// No tray on this platform; the app behaves as before.
    pub struct Tray;

    impl Tray {
        pub fn new(_cc: &eframe::CreationContext<'_>, _language: Language) -> Option<Self> {
            None
        }

        pub fn set_language(&mut self, _language: Language) {}

        pub fn quitting(&self) -> bool {
            false
        }

        pub fn hide_window(&self) {}

        pub fn window_visible(&self) -> bool {
            true
        }
    }
}
