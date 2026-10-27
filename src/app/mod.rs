//! The main window: layout, state shared by the views, and the frame loop.

mod calibration;
mod chrome;
mod controls;
mod panels;
mod performance;
mod preview;
mod rows;
mod settings;
mod style;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, Stroke, TextureHandle, Vec2,
};
use parking_lot::Mutex;

use crate::camera::{self, CameraEntry};
use crate::config::{self, Settings};
use crate::i18n::{Language, tr};
use crate::mapping::Mapper;
use crate::perf::{self, FrameTimes, Path, Stage, ms_since};
use crate::processing::Profile;
use crate::tracker::{Command, Shared, Snapshot, Status};
use crate::ui::theme::{CustomColors, Palette, Scheme};
use crate::ui::widgets;
use crate::updater::Release;
use style::{FLAGS, apply_style, load_fonts, load_png};

const RESOLUTIONS: [(u32, u32); 4] = [(640, 480), (848, 480), (1280, 720), (1920, 1080)];
const CAMERA_RATES: [u32; 2] = [30, 60];
const ROW_HEIGHT: f32 = 30.0;
const SIDEBAR_WIDTH: f32 = 460.0;
/// Smallest window the layout holds together in.
pub const MIN_WINDOW: Vec2 = Vec2::new(1220.0, 660.0);
/// How long a row stays hovered before the rows linked to it scroll into view.
const REVEAL_AFTER: Duration = Duration::from_millis(250);
const X_PROFILE: &str = "https://x.com/setsumi_vrc";

#[derive(Default)]
enum UpdateState {
    #[default]
    Idle,
    Checking,
    Ready(Vec<Release>),
    Downloading,
    Failed(String),
}

/// The row under the pointer in one of the three parameter panels.
#[derive(Clone, Copy, PartialEq)]
enum Hover {
    Channel(usize),
    Output(usize),
    Avatar(usize),
}

/// Where this frame's rows were drawn, for the links between the panels.
struct Rows {
    channels: Vec<(usize, Rect)>,
    outputs: Vec<(usize, Rect)>,
    avatar: Vec<(usize, Rect)>,
    /// Scrolled area of the MediaPipe, Unified Expressions and Avatar panels.
    view: [Rect; 3],
    /// Outer frame of the same panels.
    frame: [Rect; 3],
}

impl Default for Rows {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            outputs: Vec::new(),
            avatar: Vec::new(),
            view: [Rect::NOTHING; 3],
            frame: [Rect::NOTHING; 3],
        }
    }
}

impl Rows {
    /// Vertical centre of a row, kept inside its panel so rows scrolled out of view
    /// still get a link pointing at the edge.
    fn y(list: &[(usize, Rect)], index: usize, view: Rect) -> Option<f32> {
        list.iter().find(|(i, _)| *i == index).map(|(_, r)| r.center().y.min(view.bottom()).max(view.top()))
    }

    /// Which way a row lies when it is scrolled out of its panel: -1 above, 1 below, 0 in view.
    fn off(list: &[(usize, Rect)], index: usize, view: Rect) -> f32 {
        match list.iter().find(|(i, _)| *i == index).map(|(_, r)| r.center().y) {
            Some(y) if y > view.bottom() => 1.0,
            Some(y) if y < view.top() => -1.0,
            _ => 0.0,
        }
    }
}

pub struct App {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    model_dir: PathBuf,
    cameras: Vec<CameraEntry>,
    texture: Option<TextureHandle>,
    /// App icon shown next to the title.
    mark: Option<TextureHandle>,
    /// The restored window size was checked against `MIN_WINDOW`.
    size_checked: bool,
    /// GitHub and X marks for the links in the settings.
    links: Vec<(TextureHandle, &'static str)>,
    /// Flag per language code, for the language menu.
    flags: Vec<(&'static str, TextureHandle)>,
    texture_seq: u64,
    search: String,
    settings_open: bool,
    expanded: Option<usize>,
    /// The MediaPipe list's filters: a group of [`channels::CATEGORIES`] (0 is all) and only
    /// the parameters changed from their defaults.
    filter_group: usize,
    filter_modified: bool,
    /// The profile last loaded or saved, and the name typed for the next save.
    profile_active: Option<String>,
    profile_name: String,
    /// Delete was pressed and waits for a yes or a no.
    profile_confirm: bool,
    /// The same for Reset all.
    reset_confirm: bool,
    /// What "modified" is measured against: the saved profile in use, or the defaults.
    baseline: Profile,
    expanded_bindings: BTreeSet<usize>,
    mapper: Mapper,
    pal: Palette,
    applied_style: Option<(Scheme, CustomColors, f32)>,
    saved: Settings,
    last_save: Instant,
    last_activity: Instant,
    updates: Arc<Mutex<UpdateState>>,
    selected_release: Option<String>,
    erase_armed: Option<Instant>,
    data_size: (u64, Instant),
    rows: Rows,
    /// Hover from the previous frame, which decides this frame's highlights.
    hover: Option<Hover>,
    hover_now: Option<Hover>,
    /// Row positions from the previous frame.
    prev_rows: Rows,
    /// The hovered row and since when, so linked rows only scroll into view on a pause.
    dwell: (Option<Hover>, Instant),
    /// Per panel, a row was already scrolled into view for the hovered row.
    revealed: [bool; 3],
    /// When the pointer started resting on the blurred preview.
    reveal_started: Option<Instant>,
    /// Narrow layout: Unified Expressions above Avatar instead of side by side.
    narrow: bool,
    /// Last tracker frame counted in the interface latencies.
    seen_frame: u64,
    /// Language the CJK font order was built for.
    font_language: Language,
    /// Tray icon; closing the window hides it there instead of quitting.
    tray: Option<crate::ui::tray::Tray>,
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Shared>,
        commands: Sender<Command>,
        model_dir: PathBuf,
    ) -> Self {
        let saved = shared.settings.read().clone();
        let tray = crate::ui::tray::Tray::new(cc, shared.clone());
        let language = saved.language;
        load_fonts(&cc.egui_ctx, language);
        let pal = Palette::of(saved.scheme, &saved.custom_colors);
        Self {
            shared,
            commands,
            model_dir,
            cameras: camera::list(),
            texture: None,
            mark: load_png(&cc.egui_ctx, "mark", include_bytes!("../../assets/icon.png")),
            size_checked: false,
            links: [
                ("github", include_bytes!("../../assets/github.png").as_slice(), env!("CARGO_PKG_REPOSITORY")),
                ("x", include_bytes!("../../assets/x.png").as_slice(), X_PROFILE),
            ]
            .into_iter()
            .filter_map(|(name, png, url)| Some((load_png(&cc.egui_ctx, name, png)?, url)))
            .collect(),
            flags: FLAGS.iter().filter_map(|(code, png)| Some((*code, load_png(&cc.egui_ctx, code, png)?))).collect(),
            texture_seq: 0,
            search: String::new(),
            settings_open: false,
            expanded: None,
            filter_group: 0,
            filter_modified: false,
            profile_active: None,
            profile_name: String::new(),
            profile_confirm: false,
            reset_confirm: false,
            baseline: Profile::default(),
            expanded_bindings: BTreeSet::new(),
            mapper: Mapper::new(),
            pal,
            applied_style: None,
            saved,
            last_save: Instant::now(),
            last_activity: Instant::now(),
            updates: Arc::default(),
            selected_release: None,
            erase_armed: None,
            data_size: (config::dir_size(&config::data_dir()), Instant::now()),
            rows: Rows::default(),
            hover: None,
            hover_now: None,
            prev_rows: Rows::default(),
            dwell: (None, Instant::now()),
            revealed: [false; 3],
            reveal_started: None,
            narrow: false,
            seen_frame: 0,
            font_language: language,
            tray,
        }
    }

    fn t(&self, s: &Settings, key: &str) -> &'static str {
        tr(s.language, key)
    }

    fn persist(&mut self) {
        if self.last_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_save = Instant::now();
        let current = self.shared.settings.read().clone();
        if current != self.saved {
            if let Err(e) = current.save() {
                log::warn!("saving settings: {e:#}");
            }
            self.saved = current;
        }
        if self.shared.calibrations_dirty.swap(false, Ordering::Relaxed) {
            let store = self.shared.calibrations.lock().clone();
            if let Err(e) = config::save_calibrations(&store) {
                log::warn!("saving calibrations: {e:#}");
            }
        }
    }

    /// Color and words of the status pill: the one place that says whether tracking works.
    fn status_line(&self, s: &Settings, snapshot: &Snapshot) -> (Color32, String) {
        status_line(&self.pal, s.language, s.ft_output, &snapshot.status, snapshot.tracking_fps)
    }

    /// The first view: the camera as large as the window allows next to the calibration.
    fn simple_view(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, compact: bool) {
        if compact {
            egui::ScrollArea::vertical().id_salt("simple").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                self.camera_card(ui, s, snapshot, false);
                self.calibration_card(ui, s, snapshot, true);
                let size = Vec2::new(ui.available_width(), 420.0);
                ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(size);
                    self.mediapipe_panel(ui, s, snapshot, true);
                });
            });
            return;
        }
        let gap = 16.0;
        let height = ui.available_height();
        let main = ui.available_width() - SIDEBAR_WIDTH - gap;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let column = Layout::top_down(Align::Min);
            ui.allocate_ui_with_layout(Vec2::new(main, height), column, |ui| self.camera_card(ui, s, snapshot, true));
            ui.allocate_ui_with_layout(Vec2::new(SIDEBAR_WIDTH, height), column, |ui| {
                self.calibration_card(ui, s, snapshot, true);
                ui.add_space(12.0);
                // The rest of the column: every MediaPipe parameter, to search and calibrate.
                let size = Vec2::new(SIDEBAR_WIDTH, ui.available_height().max(160.0));
                ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(size);
                    self.mediapipe_panel(ui, s, snapshot, true);
                });
            });
        });
    }

    /// Adds a preview drawn for the first time to the Preview latency.
    fn record_preview(&self, times: Option<FrameTimes>, draw_started: Instant) {
        if let Some(times) = times {
            let breakdown = perf::interface_breakdown(&times, draw_started, Instant::now());
            self.shared.perf.lock().record_path(Path::Preview, breakdown);
        }
    }

    fn sleeping(&self, s: &Settings) -> bool {
        s.idle_sleep && self.last_activity.elapsed() > Duration::from_secs(s.idle_minutes as u64 * 60)
    }
}

/// Color and words for the tracking state, shared by the status pill and the tray menu.
pub fn status_line(pal: &Palette, language: Language, ft_output: bool, status: &Status, fps: f32) -> (Color32, String) {
    if !ft_output {
        return (pal.warn, tr(language, "paused").to_string());
    }
    match status {
        Status::Running => (pal.ok, format!("{} · {:.0} FPS", tr(language, "tracking"), fps)),
        Status::Starting => (pal.dim, tr(language, "starting").to_string()),
        Status::NoFace => (pal.warn, tr(language, "no_face").to_string()),
        Status::CameraError(_) => (pal.err, tr(language, "camera_error").to_string()),
        Status::ModelError(_) => (pal.err, tr(language, "model_error").to_string()),
    }
}

impl eframe::App for App {
    /// Also runs while the window is hidden in the tray: keeps the tray icon's dot current.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Some(tray) = &mut self.tray else { return };
        tray.refresh(&self.pal, self.mark.as_ref().map(|m| m.id()));
        if !tray.window_visible() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let draw_started = Instant::now();
        let ctx = ui.ctx().clone();
        if ctx.input(|i| !i.events.is_empty() || i.pointer.velocity() != Vec2::ZERO) {
            self.last_activity = Instant::now();
        }
        let mut s = self.shared.settings.read().clone();
        let before = s.clone();

        if let Some(tray) = &mut self.tray {
            // Before any early return: the menu window must be declared in every pass.
            tray.declare_menu(&ctx);
            if s.close_to_tray && ctx.input(|i| i.viewport().close_requested()) && !tray.quitting() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                tray.hide_window();
            }
            if !tray.window_visible() {
                // Nothing to draw; tracking and OSC go on without the interface.
                self.shared.ui_awake.store(false, Ordering::Relaxed);
                return;
            }
        }

        if self.font_language != s.language {
            load_fonts(&ctx, s.language);
            self.font_language = s.language;
        }

        let style_key = (s.scheme, s.custom_colors, s.interface_scale);
        if self.applied_style != Some(style_key) {
            self.pal = Palette::of(s.scheme, &s.custom_colors);
            apply_style(&ctx, &self.pal);
            ctx.set_zoom_factor(s.interface_scale);
            self.applied_style = Some(style_key);
        }

        if self.sleeping(&s) {
            self.shared.ui_awake.store(false, Ordering::Relaxed);
            egui::CentralPanel::default().frame(Frame::new().fill(self.pal.bg)).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.4);
                    ui.label(RichText::new(self.t(&s, "sleep_title")).heading());
                    ui.label(RichText::new(self.t(&s, "sleep_body")).color(self.pal.dim));
                });
            });
            ctx.request_repaint_after(Duration::from_secs(1));
            self.persist();
            return;
        }
        self.shared.ui_awake.store(true, Ordering::Relaxed);

        let copy_started = Instant::now();
        let snapshot = self.shared.snapshot.lock().clone();
        self.shared.perf.lock().record(Stage::Snapshot, ms_since(copy_started));
        // A frame drawn for the first time adds to the Tables and Preview latencies.
        let new_frame = (snapshot.frame_seq != self.seen_frame).then_some(snapshot.times).flatten();
        self.seen_frame = snapshot.frame_seq;
        // Uploaded even without a tracked face; only a new frame's image adds to the Preview latency.
        let uploaded = self.update_texture(&ctx, &snapshot);
        let new_preview = new_frame.filter(|_| uploaded);
        self.hover = self.hover_now.take();
        self.prev_rows = std::mem::take(&mut self.rows);
        if self.dwell.0 != self.hover {
            self.dwell = (self.hover, Instant::now());
            self.revealed = [false; 3];
        }
        if self.hover.is_some() && self.dwell.1.elapsed() < REVEAL_AFTER {
            ctx.request_repaint_after(REVEAL_AFTER);
        }

        // Too narrow for a sidebar: the camera and calibration move to the top of the content.
        let compact = ui.available_width() < 760.0;
        let bar = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 10));
        let top = egui::Panel::top("top").frame(bar).show(ui, |ui| self.top_bar(ui, &mut s, &snapshot));
        let edge = top.response.rect;
        ui.painter().hline(edge.x_range(), edge.bottom() - 0.5, Stroke::new(1.0, self.pal.line));
        self.window_edges(&ctx);
        let advanced = s.advanced;
        let body = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 14));
        egui::CentralPanel::default().frame(body).show(ui, |ui| {
            if !advanced {
                self.simple_view(ui, &mut s, &snapshot, compact);
                self.record_preview(new_preview, draw_started);
                return;
            }
            if !compact {
                self.camera_strip(ui, &mut s, &snapshot);
                ui.add_space(8.0);
                self.record_preview(new_preview, draw_started);
            } else {
                let title = format!("{} · {}", self.t(&s, "camera"), self.t(&s, "calibrate"));
                let title = widgets::title(&self.pal, &title);
                egui::CollapsingHeader::new(title).default_open(false).show(ui, |ui| {
                    // The two cards side by side, as wide as the sidebar would make them.
                    let width = ((ui.available_width() - 12.0) / 2.0).floor();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let column = Layout::top_down(Align::Min);
                        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), column, |ui| {
                            self.camera_card(ui, &mut s, &snapshot, false);
                        });
                        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), column, |ui| {
                            self.calibration_card(ui, &mut s, &snapshot, false);
                        });
                    });
                });
                ui.add_space(8.0);
            }
            self.tools_row(ui, &mut s, &snapshot);
            ui.add_space(12.0);

            let height = (ui.available_height() - 4.0).max(120.0);
            let width = ui.available_width();
            // Wide enough for the links between the panels to be readable.
            let gap = 26.0;
            // Below this width three columns get too narrow to read; the Unified Expressions
            // and Avatar panels then share the right column, one above the other.
            self.narrow = width < 900.0;
            let mp = if self.narrow { (width - gap) * 0.5 } else { (width - gap * 2.0) * 0.48 };
            let column = |ui: &mut egui::Ui, size: Vec2, add: &mut dyn FnMut(&mut egui::Ui)| {
                ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(size);
                    add(ui);
                });
            };
            let tables_started = Instant::now();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                column(ui, Vec2::new(mp, height), &mut |ui| self.mediapipe_panel(ui, &mut s, &snapshot, false));
                if self.narrow {
                    let side = width - gap - mp;
                    let half = ((height - 10.0) / 2.0).floor();
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        column(ui, Vec2::new(side, half), &mut |ui| self.unified_panel(ui, &mut s, &snapshot));
                        column(ui, Vec2::new(side, half), &mut |ui| self.avatar_panel(ui, &mut s, &snapshot));
                    });
                } else {
                    let side = (width - gap * 2.0 - mp) / 2.0;
                    column(ui, Vec2::new(side, height), &mut |ui| self.unified_panel(ui, &mut s, &snapshot));
                    column(ui, Vec2::new(side, height), &mut |ui| self.avatar_panel(ui, &mut s, &snapshot));
                }
            });
            self.draw_links(ui);
            let mut perf = self.shared.perf.lock();
            perf.record(Stage::Tables, ms_since(tables_started));
            if let Some(times) = new_frame {
                perf.record_path(Path::Tables, perf::interface_breakdown(&times, draw_started, Instant::now()));
            }
        });
        if self.settings_open {
            let frame = Frame::new()
                .fill(self.pal.panel)
                .stroke(Stroke::new(1.0, self.pal.line))
                .corner_radius(CornerRadius::same(widgets::RADIUS + 6))
                .inner_margin(Margin::same(20));
            let modal = egui::Modal::new(ui.make_persistent_id("settings_modal"))
                .frame(frame)
                .backdrop_color(Color32::from_black_alpha(150))
                .show(&ctx, |ui| {
                    ui.set_width(460.0);
                    self.settings_panel(ui, &mut s, &snapshot);
                });
            if modal.should_close() {
                self.settings_open = false;
            }
        }
        if self.hover_now != self.hover {
            // Highlights follow the previous frame's hover; draw once more to catch up.
            ctx.request_repaint();
        }
        if s.debug {
            self.performance_window(&ctx, &mut s);
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        if s != before {
            // The tracker may have updated `last_avatar_id` while we drew; keep its value.
            let mut shared = self.shared.settings.write();
            let last_avatar = shared.last_avatar_id.clone();
            *shared = s.clone();
            shared.last_avatar_id = last_avatar;
            self.shared.receive_port.store(s.receive_port, Ordering::Relaxed);
        }
        self.shared.perf.lock().record(Stage::Draw, ms_since(draw_started));
        self.persist();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let settings = self.shared.settings.read().clone();
        if let Err(e) = settings.save() {
            log::warn!("saving settings: {e:#}");
        }
        let store = self.shared.calibrations.lock().clone();
        if let Err(e) = config::save_calibrations(&store) {
            log::warn!("saving calibrations: {e:#}");
        }
    }
}
