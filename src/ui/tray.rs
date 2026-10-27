//! System tray icon. Closing the window hides it there (when enabled in the settings) while
//! tracking and OSC keep running. Right-clicking the icon opens a menu drawn by the app in the
//! scheme colors ("Open", "Pause" like the FT switch, "Quit"), and a dot on the icon shows the
//! tracking state. Windows only: other platforms have no tray.

#[cfg(windows)]
pub use imp::Tray;

#[cfg(not(windows))]
pub use stub::Tray;

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::time::Duration;

    use eframe::egui::{
        self, Align2, Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, Sense, Stroke, TextureId, Vec2,
        ViewportBuilder, ViewportId,
    };
    use parking_lot::Mutex;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    use windows::Win32::UI::Controls::MARGINS;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetWindowRect, HWND_TOPMOST, IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_NOSIZE,
        SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, ShowWindow,
    };
    use windows::core::{HSTRING, PCWSTR};

    use crate::app::status_line;
    use crate::i18n::tr;
    use crate::tracker::Shared;
    use crate::ui::icons::{self, Glyph};
    use crate::ui::theme::{Palette, mix};
    use crate::ui::widgets;

    /// Window title the menu is found by (its handle is not exposed to the callback).
    const MENU_TITLE: &str = "VRC Openface menu";
    const MENU_SIZE: Vec2 = Vec2::new(236.0, 184.0);

    pub struct Tray {
        // Dropping the icon removes it from the tray.
        icon: TrayIcon,
        base: Icon,
        base_rgba: (Vec<u8>, u32, u32),
        badge: Option<Color32>,
        /// Status text the tooltip shows.
        tooltip: String,
        menu: Arc<Menu>,
        shared: Arc<Shared>,
        hwnd: isize,
        quitting: Arc<AtomicBool>,
    }

    /// The right-click menu: a small borderless window drawn by the app in the scheme colors.
    /// Win32 calls show and hide it because the UI loop may be idle while the main window is
    /// hidden.
    struct Menu {
        shared: Arc<Shared>,
        main: isize,
        quitting: Arc<AtomicBool>,
        pal: Mutex<Palette>,
        mark: Mutex<Option<TextureId>>,
        /// Handle of the menu window, 0 until it is found.
        hwnd: AtomicIsize,
        open: AtomicBool,
        /// Focus is reported a moment after showing; closing on focus loss waits for it.
        focused_once: AtomicBool,
    }

    fn handle(hwnd: isize) -> HWND {
        HWND(hwnd as *mut c_void)
    }

    fn menu_id() -> ViewportId {
        ViewportId::from_hash_of("tray_menu")
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

    /// Draws a status dot in the bottom right corner of the tray icon.
    fn with_badge(base: &(Vec<u8>, u32, u32), dot: Color32, ring: Color32) -> Vec<u8> {
        let (rgba, width, height) = base;
        let mut out = rgba.clone();
        let radius = *width as f32 * 0.2;
        let centre = Pos2::new(*width as f32 - radius * 1.2, *height as f32 - radius * 1.2);
        for y in 0..*height {
            for x in 0..*width {
                let d = Pos2::new(x as f32 + 0.5, y as f32 + 0.5).distance(centre);
                let i = ((y * width + x) * 4) as usize;
                for (r, color) in [(radius * 1.3, ring), (radius, dot)] {
                    let a = (r + 0.5 - d).clamp(0.0, 1.0);
                    if a <= 0.0 {
                        continue;
                    }
                    let below = out[i + 3] as f32 / 255.0;
                    let alpha = a + below * (1.0 - a);
                    for (k, c) in [color.r(), color.g(), color.b()].into_iter().enumerate() {
                        out[i + k] = ((c as f32 * a + out[i + k] as f32 * below * (1.0 - a)) / alpha) as u8;
                    }
                    out[i + 3] = (alpha * 255.0) as u8;
                }
            }
        }
        out
    }

    impl Menu {
        fn window(&self) -> Option<HWND> {
            let mut hwnd = self.hwnd.load(Ordering::Relaxed);
            if hwnd == 0 {
                let found = unsafe { FindWindowW(PCWSTR::null(), &HSTRING::from(MENU_TITLE)) }.ok()?;
                hwnd = found.0 as isize;
                self.hwnd.store(hwnd, Ordering::Relaxed);
                // Rounded corners and a shadow, where the system has them (Windows 11).
                unsafe {
                    let round = DWMWCP_ROUND;
                    let _ = DwmSetWindowAttribute(
                        found,
                        DWMWA_WINDOW_CORNER_PREFERENCE,
                        &round as *const _ as *const c_void,
                        size_of_val(&round) as u32,
                    );
                    let margins = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
                    let _ = DwmExtendFrameIntoClientArea(found, &margins);
                }
            }
            Some(handle(hwnd))
        }

        /// Shows the menu above (or below) the tray icon, inside the work area of its monitor.
        fn open_at(&self, ctx: &egui::Context, x: i32, y: i32) {
            let Some(hwnd) = self.window() else {
                // No menu window: the main window is the way in.
                show(self.main, ctx);
                return;
            };
            unsafe {
                let mut size = RECT::default();
                let _ = GetWindowRect(hwnd, &mut size);
                let (width, height) = (size.right - size.left, size.bottom - size.top);
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
                let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                    info.rcWork
                } else {
                    RECT { left: 0, top: 0, right: x + width, bottom: y + height }
                };
                let gap = 8;
                let left = (x - width / 2).min(work.right - width - gap).max(work.left + gap);
                let top = if y > (work.top + work.bottom) / 2 {
                    (y - height).min(work.bottom - height) - gap
                } else {
                    y.max(work.top) + gap
                };
                self.focused_once.store(false, Ordering::Relaxed);
                self.open.store(true, Ordering::Relaxed);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    left,
                    top.max(work.top + gap),
                    0,
                    0,
                    SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                let _ = SetForegroundWindow(hwnd);
            }
            ctx.request_repaint_of(menu_id());
        }

        fn hide(&self) {
            self.open.store(false, Ordering::Relaxed);
            if let Some(hwnd) = self.window() {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }

        fn toggle_pause(&self, ctx: &egui::Context) {
            // The main window may be hidden, so the setting is changed and saved here instead
            // of by the UI.
            let settings = {
                let mut settings = self.shared.settings.write();
                settings.ft_output = !settings.ft_output;
                settings.clone()
            };
            if let Err(e) = settings.save() {
                log::warn!("saving settings: {e:#}");
            }
            ctx.request_repaint();
        }

        fn draw(&self, ui: &mut egui::Ui) {
            let ctx = ui.ctx().clone();
            if self.open.load(Ordering::Relaxed) {
                match ctx.input(|i| i.viewport().focused) {
                    Some(true) => self.focused_once.store(true, Ordering::Relaxed),
                    Some(false) if self.focused_once.load(Ordering::Relaxed) => self.hide(),
                    _ => {}
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.hide();
                }
                // The status line follows tracking while the menu is open.
                ctx.request_repaint_after_for(Duration::from_millis(250), menu_id());
            }
            let pal = *self.pal.lock();
            let (language, ft_output) = {
                let settings = self.shared.settings.read();
                (settings.language, settings.ft_output)
            };
            let (color, status) = {
                let snapshot = self.shared.snapshot.lock();
                status_line(&pal, language, ft_output, &snapshot.status, snapshot.tracking_fps)
            };
            let frame = Frame::new().fill(pal.panel).stroke(Stroke::new(1.0, pal.line)).inner_margin(Margin::same(8));
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let width = ui.available_width();

                let (head, _) = ui.allocate_exact_size(Vec2::new(width, 46.0), Sense::hover());
                let painter = ui.painter();
                if let Some(mark) = *self.mark.lock() {
                    let at = Rect::from_min_size(head.min + Vec2::new(6.0, 9.0), Vec2::splat(28.0));
                    painter.image(mark, at, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                painter.text(
                    head.min + Vec2::new(44.0, 15.0),
                    Align2::LEFT_CENTER,
                    "VRC Openface",
                    FontId::new(13.5, widgets::semibold()),
                    pal.text,
                );
                painter.circle_filled(head.min + Vec2::new(47.0, 33.0), 3.5, color);
                painter.text(
                    head.min + Vec2::new(57.0, 33.0),
                    Align2::LEFT_CENTER,
                    status,
                    FontId::proportional(11.5),
                    pal.dim,
                );
                separator(ui, &pal);

                if item(ui, &pal, Glyph::Face, tr(language, "tray_open"), false).clicked() {
                    show(self.main, &ctx);
                    self.hide();
                }
                let (glyph, label) = if ft_output {
                    (Glyph::Pause, tr(language, "tray_pause"))
                } else {
                    (Glyph::Play, tr(language, "resume"))
                };
                if item(ui, &pal, glyph, label, false).clicked() {
                    self.toggle_pause(&ctx);
                    self.hide();
                }
                separator(ui, &pal);
                if item(ui, &pal, Glyph::Close, tr(language, "tray_quit"), true).clicked() {
                    // The window is shown first so its loop is running to handle the close.
                    self.quitting.store(true, Ordering::Relaxed);
                    show(self.main, &ctx);
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Close);
                    self.hide();
                }
            });
        }
    }

    fn separator(ui: &mut egui::Ui, pal: &Palette) {
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::ZERO, pal.line.gamma_multiply(0.8));
        ui.add_space(4.0);
    }

    /// One row of the menu; `danger` colors it as the destructive choice.
    fn item(ui: &mut egui::Ui, pal: &Palette, glyph: Glyph, label: &str, danger: bool) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let hot = response.hovered();
        let painter = ui.painter();
        if hot {
            let fill = if danger { mix(pal.panel, pal.err, 0.2) } else { pal.raised };
            painter.rect_filled(rect, CornerRadius::same(widgets::RADIUS), fill);
        }
        let fg = if danger && hot { pal.err } else { pal.text };
        let area = Rect::from_center_size(rect.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0));
        icons::paint(painter, glyph, area, if hot { fg } else { fg.gamma_multiply(0.8) });
        painter.text(
            rect.left_center() + Vec2::new(36.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            fg,
        );
        response
    }

    impl Tray {
        pub fn new(cc: &eframe::CreationContext<'_>, shared: Arc<Shared>) -> Option<Self> {
            let hwnd = match cc.window_handle().ok()?.as_raw() {
                RawWindowHandle::Win32(window) => window.hwnd.get(),
                _ => return None,
            };
            let image = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon.png")).ok()?;
            let base_rgba = (image.rgba.clone(), image.width, image.height);
            let base = Icon::from_rgba(image.rgba, image.width, image.height).ok()?;
            let tray = TrayIconBuilder::new()
                .with_icon(base.clone())
                .with_tooltip("VRC Openface")
                .build()
                .map_err(|e| log::warn!("tray icon: {e}"))
                .ok()?;

            let ctx = cc.egui_ctx.clone();
            let quitting = Arc::new(AtomicBool::new(false));
            let pal = {
                let settings = shared.settings.read();
                Palette::of(settings.scheme, &settings.custom_colors)
            };
            let menu = Arc::new(Menu {
                shared: shared.clone(),
                main: hwnd,
                quitting: quitting.clone(),
                pal: Mutex::new(pal),
                mark: Mutex::new(None),
                hwnd: AtomicIsize::new(0),
                open: AtomicBool::new(false),
                focused_once: AtomicBool::new(false),
            });
            let handler_menu = menu.clone();
            TrayIconEvent::set_event_handler(Some(move |event| {
                if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, position, .. } = event {
                    match button {
                        MouseButton::Left => show(hwnd, &ctx),
                        MouseButton::Right => handler_menu.open_at(&ctx, position.x as i32, position.y as i32),
                        _ => {}
                    }
                }
            }));
            Some(Self {
                icon: tray,
                base,
                base_rgba,
                badge: None,
                tooltip: String::new(),
                menu,
                shared,
                hwnd,
                quitting,
            })
        }

        /// Declares the menu window; called in every UI pass, or egui drops the window.
        pub fn declare_menu(&self, ctx: &egui::Context) {
            let menu = self.menu.clone();
            let builder = ViewportBuilder::default()
                .with_title(MENU_TITLE)
                .with_inner_size(MENU_SIZE)
                .with_decorations(false)
                .with_resizable(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_visible(false);
            ctx.show_viewport_deferred(menu_id(), builder, move |ui, _| menu.draw(ui));
        }

        /// Hands the menu the colors and keeps the icon's status dot and tooltip current. Runs
        /// from `App::logic`, which also ticks while the window is hidden.
        pub fn refresh(&mut self, pal: &Palette, mark: Option<TextureId>) {
            *self.menu.pal.lock() = *pal;
            *self.menu.mark.lock() = mark;
            let (color, text) = {
                let settings = self.shared.settings.read();
                let snapshot = self.shared.snapshot.lock();
                status_line(pal, settings.language, settings.ft_output, &snapshot.status, snapshot.tracking_fps)
            };
            if self.badge != Some(color) {
                self.badge = Some(color);
                let rgba = with_badge(&self.base_rgba, color, pal.bg);
                let icon =
                    Icon::from_rgba(rgba, self.base_rgba.1, self.base_rgba.2).unwrap_or_else(|_| self.base.clone());
                let _ = self.icon.set_icon(Some(icon));
            }
            if self.tooltip != text {
                let _ = self.icon.set_tooltip(Some(format!("VRC Openface · {text}")));
                self.tooltip = text;
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
    use std::sync::Arc;

    use crate::tracker::Shared;

    /// No tray on this platform; the app behaves as before.
    pub struct Tray;

    impl Tray {
        pub fn new(_cc: &eframe::CreationContext<'_>, _shared: Arc<Shared>) -> Option<Self> {
            None
        }

        pub fn declare_menu(&self, _ctx: &egui::Context) {}

        pub fn refresh(&mut self, _pal: &crate::ui::theme::Palette, _mark: Option<egui::TextureId>) {}

        pub fn quitting(&self) -> bool {
            false
        }

        pub fn hide_window(&self) {}

        pub fn window_visible(&self) -> bool {
            true
        }
    }
}
