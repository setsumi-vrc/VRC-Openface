use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect,
    RichText, Sense, Shape, Stroke, StrokeKind, TextStyle, TextureHandle, TextureOptions, Vec2,
};
use parking_lot::Mutex;

use crate::camera::{self, CameraEntry};
use crate::channels::{self, COUNT};
use crate::config::{self, Device, IDLE_MINUTES, Settings};
use crate::i18n::{Language, tr};
use crate::mapping::{Mapper, OUTPUTS};
use crate::osc::Binding;
use crate::perf::Stage;
use crate::processing::{self, ChannelCalibration, Source};
use crate::theme::{CustomColors, Palette, Scheme};
use crate::tracker::{self, Command, Shared, Snapshot, Status, TongueStatus};
use crate::updater::{self, Channel, Release};

const RESOLUTIONS: [(u32, u32); 4] = [(640, 480), (848, 480), (1280, 720), (1920, 1080)];
const SCALES: [f32; 6] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5];
const MAPPING_DOCS: &str = "https://docs.vrcft.io/docs/tutorial-avatars/tutorial-avatars-extras/parameters";
const ROW_HEIGHT: f32 = 30.0;
const X_PROFILE: &str = "https://x.com/setsumi_vrc";
/// How long the pointer rests on a blurred preview before it is shown unblurred, and
/// for how long.
const REVEAL_HOLD: Duration = Duration::from_millis(1500);
const REVEAL_FOR: Duration = Duration::from_secs(5 * 60);

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
}

enum Icon<'a> {
    Image(egui::TextureId),
    Glyph(&'a str),
}

/// Rows connected to the hovered one, highlighted in every panel.
struct Related {
    channels: [bool; COUNT],
    outputs: Vec<bool>,
}

pub struct App {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    model_dir: PathBuf,
    cameras: Vec<CameraEntry>,
    texture: Option<TextureHandle>,
    /// GitHub and X marks for the links next to the title.
    links: Vec<(TextureHandle, &'static str)>,
    texture_seq: u64,
    search: String,
    settings_open: bool,
    theme_open: bool,
    expanded: Option<usize>,
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
    /// While a channel stays hovered: where its expressions were moved to in the
    /// Unified Expressions list (`None` when they were all visible and stay in place).
    pulled: Option<(usize, Option<usize>)>,
    /// When the pointer started resting on the blurred preview.
    reveal_started: Option<Instant>,
    /// Narrow layout: Unified Expressions above Avatar instead of side by side.
    narrow: bool,
}

fn load_png(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(png).map_err(|e| log::warn!("{name} icon: {e}")).ok()?;
    let image = ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

/// Dashed outline used for panels and channel groups.
fn dashed_rect(painter: &egui::Painter, rect: Rect, color: Color32) {
    let r = rect.shrink(0.5);
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(Shape::dashed_line(&corners, Stroke::new(1.0, color), 4.0, 3.0));
}

/// Adds Windows' Japanese and Korean fonts as fallbacks so every language renders.
fn load_cjk_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    let fonts = std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")).unwrap_or_default();
    let japanese = ["YuGothM.ttc", "msgothic.ttc"].iter().find_map(|f| std::fs::read(fonts.join(f)).ok());
    let korean = std::fs::read(fonts.join("malgun.ttf")).ok();
    for (name, bytes) in [("japanese", japanese), ("korean", korean)] {
        let Some(bytes) = bytes else { continue };
        let families = [FontFamily::Proportional, FontFamily::Monospace]
            .into_iter()
            .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest })
            .collect();
        ctx.add_font(FontInsert { name: name.into(), data: egui::FontData::from_owned(bytes), families });
    }
}

fn apply_style(ctx: &egui::Context, pal: &Palette) {
    ctx.global_style_mut(|style| {
        for (text_style, size) in [
            (TextStyle::Body, 12.5),
            (TextStyle::Button, 12.5),
            (TextStyle::Monospace, 12.5),
            (TextStyle::Small, 10.5),
            (TextStyle::Heading, 17.0),
        ] {
            style.text_styles.insert(text_style, FontId::new(size, FontFamily::Monospace));
        }
        let mut v = if pal.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = pal.bg;
        v.window_fill = pal.panel;
        v.extreme_bg_color = pal.bg;
        v.faint_bg_color = pal.panel;
        v.override_text_color = Some(pal.text);
        v.selection.bg_fill = pal.accent;
        v.selection.stroke = Stroke::new(1.0, pal.on_accent);
        v.window_stroke = Stroke::new(1.0, pal.line);
        v.hyperlink_color = pal.accent;
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::ZERO;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.line);
        v.widgets.inactive.bg_fill = pal.panel;
        v.widgets.inactive.weak_bg_fill = pal.bg;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.line);
        v.widgets.hovered.weak_bg_fill = pal.panel;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.dim);
        v.widgets.active.bg_fill = pal.accent;
        style.visuals = v;
        style.spacing.button_padding = Vec2::new(8.0, 5.0);
        style.spacing.item_spacing = Vec2::new(6.0, 4.0);
        style.spacing.slider_width = 140.0;
    });
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

/// Panel header: the title on the left, truncated to the room the controls on the right
/// leave. When that room gets too small the controls move to a second line.
fn header(ui: &mut egui::Ui, id: &str, title: impl FnOnce(&mut egui::Ui), controls: impl FnOnce(&mut egui::Ui)) {
    let key = egui::Id::unique(("header-controls", id));
    let needed: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(0.0);
    let right_to_left = Layout::right_to_left(Align::Center);
    let measured = if needed + 150.0 > ui.available_width() {
        ui.horizontal(|ui| ui.with_layout(Layout::left_to_right(Align::Center), title));
        // Wrapping keeps the controls inside the panel; overflowing a right-to-left layout
        // would push the whole panel off to the left.
        let wrapping = Layout::right_to_left(Align::Center).with_main_wrap(true);
        // Start one row high so the rows are not centred in the whole panel.
        let size = Vec2::new(ui.available_width(), ui.spacing().interact_size.y);
        let rect = ui.allocate_ui_with_layout(size, wrapping, |ui| ui.scope(controls).response.rect).inner;
        // A wrapped row is narrower than the controls really are; keep the old width then.
        (rect.height() < ui.spacing().interact_size.y * 1.6).then_some(rect.width())
    } else {
        ui.horizontal(|ui| {
            ui.with_layout(right_to_left, |ui| {
                let width = ui.scope(controls).response.rect.width();
                ui.with_layout(Layout::left_to_right(Align::Center), title);
                Some(width)
            })
            .inner
        })
        .inner
    };
    if let Some(width) = measured {
        ui.data_mut(|d| d.insert_temp(key, width));
    }
}

/// Single-line text cut with "…" to fit `width`.
fn truncated(painter: &egui::Painter, text: String, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    painter.layout_job(job)
}

fn mib(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Shared>,
        commands: Sender<Command>,
        model_dir: PathBuf,
    ) -> Self {
        load_cjk_fonts(&cc.egui_ctx);
        let saved = shared.settings.read().clone();
        let pal = Palette::of(saved.scheme, &saved.custom_colors);
        Self {
            shared,
            commands,
            model_dir,
            cameras: camera::list(),
            texture: None,
            links: [
                ("github", include_bytes!("../assets/github.png").as_slice(), env!("CARGO_PKG_REPOSITORY")),
                ("x", include_bytes!("../assets/x.png").as_slice(), X_PROFILE),
            ]
            .into_iter()
            .filter_map(|(name, png, url)| Some((load_png(&cc.egui_ctx, name, png)?, url)))
            .collect(),
            texture_seq: 0,
            search: String::new(),
            settings_open: false,
            theme_open: false,
            expanded: None,
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
            pulled: None,
            reveal_started: None,
            narrow: false,
        }
    }

    fn related(&self) -> Related {
        let mut related = Related { channels: [false; COUNT], outputs: vec![false; OUTPUTS.len()] };
        match self.hover {
            Some(Hover::Channel(c)) => {
                related.channels[c] = true;
                for (o, flag) in related.outputs.iter_mut().enumerate() {
                    *flag = self.mapper.dependencies(o).contains(&c);
                }
            }
            Some(Hover::Output(o) | Hover::Avatar(o)) => {
                related.outputs[o] = true;
                self.mapper.dependencies(o).iter().for_each(|&c| related.channels[c] = true);
            }
            None => {}
        }
        related
    }

    fn t(&self, s: &Settings, key: &str) -> &'static str {
        tr(s.language, key)
    }

    fn matches(&self, text: &str) -> bool {
        self.search.is_empty() || text.to_lowercase().contains(&self.search.to_lowercase())
    }

    /// Filled with the accent colour when on, like a pressed key.
    fn toggle(&self, ui: &mut egui::Ui, on: &mut bool, text: &str) -> egui::Response {
        let (fg, bg) = if *on { (self.pal.on_accent, self.pal.accent) } else { (self.pal.text, self.pal.bg) };
        let response = ui.add(egui::Button::new(RichText::new(text).color(fg)).fill(bg));
        if response.clicked() {
            *on = !*on;
        }
        response
    }

    /// Square button showing an image or a glyph: outlined on hover, filled when `on`.
    fn icon_button(&self, ui: &mut egui::Ui, icon: Icon<'_>, on: bool) -> egui::Response {
        let pal = self.pal;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let painter = ui.painter_at(rect.expand(1.0));
        let fg = if on {
            painter.rect_filled(rect, CornerRadius::ZERO, pal.accent);
            pal.on_accent
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::ZERO, pal.line.gamma_multiply(0.5));
            painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.dim), StrokeKind::Inside);
            pal.text
        } else {
            painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.line), StrokeKind::Inside);
            pal.text.gamma_multiply(0.85)
        };
        match icon {
            Icon::Image(texture) => {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture, Rect::from_center_size(rect.center(), Vec2::splat(18.0)), uv, fg);
            }
            Icon::Glyph(glyph) => {
                painter.text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(18.0), fg);
            }
        }
        response
    }

    /// A labelled box that works as a slider: click or drag across it to set `value`
    /// (0..1). The filled part shows the value; 0 reads as `zero_text`.
    fn slider_box(
        &self,
        ui: &mut egui::Ui,
        value: &mut f32,
        label: &str,
        zero_text: &str,
        width: f32,
    ) -> egui::Response {
        let pal = self.pal;
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click_and_drag());
        response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if (response.dragged() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            // Snap the first few percent to zero so "off" is easy to hit.
            let v = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let v = if v < 0.03 { 0.0 } else { (v * 100.0).round() / 100.0 };
            if v != *value {
                *value = v;
                response.mark_changed();
            }
        }
        let painter = ui.painter_at(rect);
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * *value, rect.height()));
        painter.rect_filled(fill, CornerRadius::ZERO, pal.accent.gamma_multiply(0.35));
        let border = if response.hovered() || response.dragged() { pal.dim } else { pal.line };
        painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, border), StrokeKind::Inside);
        let font = FontId::monospace(12.0);
        painter.text(rect.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, label, font.clone(), pal.text);
        let shown = if *value > 0.0 { percent(*value) } else { zero_text.to_string() };
        painter.text(rect.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, shown, font, pal.text);
        response
    }

    /// Small pill switch used in panel headers and settings.
    fn switch(&self, ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(26.0, 14.0), Sense::click());
        if response.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        let painter = ui.painter_at(rect.expand(1.0));
        let fill = if *on { self.pal.accent } else { self.pal.line };
        painter.rect_filled(rect, CornerRadius::same(7), fill);
        let knob_x = if *on { rect.right() - 7.0 } else { rect.left() + 7.0 };
        painter.circle_filled(
            Pos2::new(knob_x, rect.center().y),
            4.5,
            if *on { self.pal.on_accent } else { self.pal.bg },
        );
        response
    }

    fn labeled_switch(&self, ui: &mut egui::Ui, on: &mut bool, text: &str, hint: Option<&str>) {
        ui.horizontal(|ui| {
            let label = ui.label(text);
            if let Some(hint) = hint {
                label.on_hover_text(hint);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.switch(ui, on));
        });
    }

    fn panel_frame(&self) -> Frame {
        Frame::new().fill(self.pal.bg).inner_margin(Margin::same(8))
    }

    fn update_texture(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        let Some(frame) = &snapshot.preview else { return };
        if snapshot.preview_seq == self.texture_seq {
            return;
        }
        self.texture_seq = snapshot.preview_seq;
        let image = ColorImage::from_rgb([frame.width, frame.height], &frame.data);
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("preview", image, TextureOptions::LINEAR)),
        }
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

    fn top_bar(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        ui.horizontal(|ui| {
            let power = if s.ft_output { "● FT" } else { "○ FT" };
            let hint = self.t(s, "ft_hint");
            self.toggle(ui, &mut s.ft_output, power).on_hover_text(hint);
            ui.add_space(8.0);
            ui.label(RichText::new("VRC OPENFACE").heading().strong());
            ui.add_space(10.0);
            for (texture, url) in &self.links {
                if self.icon_button(ui, Icon::Image(texture.id()), false).on_hover_text(*url).clicked() {
                    crate::system::open_url(url);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for language in Language::ALL.iter().rev() {
                    let mut on = s.language == *language;
                    if self.toggle(ui, &mut on, language.native_name()).clicked() {
                        s.language = *language;
                    }
                }
                ui.add_space(8.0);
                if self
                    .icon_button(ui, Icon::Glyph("🎨"), self.theme_open)
                    .on_hover_text(self.t(s, "color_scheme"))
                    .clicked()
                {
                    self.theme_open = !self.theme_open;
                }
                if self
                    .icon_button(ui, Icon::Glyph("⚙"), self.settings_open)
                    .on_hover_text(self.t(s, "settings"))
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(self.pal.dim));
            });
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui, s: &Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        // Short or narrow areas give more of their height to the parameter lists.
        let share = if ui.available_height() < 700.0 || ui.available_width() < 900.0 { 0.30 } else { 0.42 };
        let height = (ui.available_height() * share).clamp(120.0, 430.0);
        let (outer, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, CornerRadius::ZERO, pal.bg);

        let (fw, fh) = snapshot.frame_size;
        let aspect = if fw > 0 && fh > 0 { fw as f32 / fh as f32 } else { 16.0 / 9.0 };
        let image_w = (outer.height() * aspect).min(outer.width());
        let image = Rect::from_center_size(outer.center(), Vec2::new(image_w, image_w / aspect));

        let hatch = Stroke::new(1.0, pal.line.gamma_multiply(0.6));
        for side in [
            Rect::from_min_max(outer.min, Pos2::new(image.left(), outer.bottom())),
            Rect::from_min_max(Pos2::new(image.right(), outer.top()), outer.max),
        ] {
            if side.width() < 1.0 {
                continue;
            }
            let side_painter = painter.with_clip_rect(side);
            let mut x = side.left() - side.height();
            while x < side.right() {
                side_painter
                    .line_segment([Pos2::new(x, side.bottom()), Pos2::new(x + side.height(), side.top())], hatch);
                x += 9.0;
            }
        }

        if s.show_camera
            && let Some(texture) = &self.texture
        {
            painter.image(texture.id(), image, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        let to_screen =
            |p: &[f32; 3]| Pos2::new(image.left() + p[0] * image.width(), image.top() + p[1] * image.height());
        if s.show_landmarks {
            let dot = if s.show_camera { pal.accent } else { pal.text };
            for p in &snapshot.landmarks {
                painter.circle_filled(to_screen(p), 0.9, dot);
            }
        }
        if s.cat_ears && snapshot.landmarks.len() > 400 {
            self.draw_ears(&painter, snapshot, &to_screen);
        }
        painter.rect_stroke(outer, CornerRadius::ZERO, Stroke::new(1.0, pal.line), StrokeKind::Inside);
        self.reveal_on_hover(ui, &painter, &response, s, image);

        let status = match &snapshot.status {
            Status::Running => None,
            Status::Starting => Some(self.t(s, "starting").to_string()),
            Status::NoFace => Some(self.t(s, "no_face").to_string()),
            Status::CameraError(e) => Some(format!("{}: {e}", self.t(s, "camera_error"))),
            Status::ModelError(e) => Some(format!("{}: {e}", self.t(s, "model_error"))),
        };
        if let Some(status) = status {
            painter.text(
                outer.left_top() + Vec2::new(14.0, 12.0),
                Align2::LEFT_TOP,
                status,
                FontId::monospace(12.0),
                pal.text,
            );
        }
        let banner = if let Some(progress) = snapshot.capture_progress {
            Some(format!("{} {:.0}%", self.t(s, "hold_still"), progress * 100.0))
        } else {
            snapshot
                .message
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
                .map(|(key, _)| self.t(s, key).to_string())
        };
        if let Some(text) = banner {
            painter.text(
                outer.center_top() + Vec2::new(0.0, 12.0),
                Align2::CENTER_TOP,
                text,
                FontId::monospace(13.0),
                pal.text,
            );
        }
        let fps = format!(
            "{} {:.0} FPS  |  {} {:.0} FPS",
            self.t(s, "tracking"),
            snapshot.tracking_fps,
            self.t(s, "camera"),
            snapshot.camera_fps
        );
        painter.text(
            outer.left_bottom() + Vec2::new(14.0, -12.0),
            Align2::LEFT_BOTTOM,
            fps,
            FontId::monospace(11.5),
            pal.dim,
        );
    }

    /// Holding the pointer over a blurred preview for `REVEAL_HOLD` shows the camera
    /// unblurred for `REVEAL_FOR`; a ring around the pointer fills while it waits.
    fn reveal_on_hover(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        response: &egui::Response,
        s: &Settings,
        image: Rect,
    ) {
        let revealed = self.shared.unblurred_until.lock().is_some_and(|until| Instant::now() < until);
        let pointer = response.hover_pos().filter(|p| image.contains(*p));
        let (Some(pointer), true, true, false) = (pointer, s.show_camera, s.camera_blur, revealed) else {
            self.reveal_started = None;
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let started = *self.reveal_started.get_or_insert_with(Instant::now);
        let progress = (started.elapsed().as_secs_f32() / REVEAL_HOLD.as_secs_f32()).min(1.0);
        if progress >= 1.0 {
            *self.shared.unblurred_until.lock() = Some(Instant::now() + REVEAL_FOR);
            self.reveal_started = None;
            return;
        }
        let center = pointer + Vec2::new(0.0, -26.0);
        let radius = 18.0;
        painter.circle_stroke(center, radius, Stroke::new(3.0, Color32::from_white_alpha(40)));
        let steps = 48;
        let arc: Vec<Pos2> = (0..=(steps as f32 * progress) as usize)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + i as f32 / steps as f32 * std::f32::consts::TAU;
                center + Vec2::angled(angle) * radius
            })
            .collect();
        painter.add(Shape::line(arc, Stroke::new(3.0, Color32::WHITE)));
        ui.ctx().request_repaint();
    }

    /// Two triangles above the forehead whose tilt follows the ear outputs.
    fn draw_ears(&self, painter: &egui::Painter, snapshot: &Snapshot, to_screen: &dyn Fn(&[f32; 3]) -> Pos2) {
        let lm = &snapshot.landmarks;
        let (forehead, chin) = (to_screen(&lm[10]), to_screen(&lm[152]));
        let (right_temple, left_temple) = (to_screen(&lm[103]), to_screen(&lm[332]));
        let up = (forehead - chin).normalized();
        let size = (forehead - chin).length() * 0.32;
        let value = |name: &str| {
            OUTPUTS.iter().position(|o| o.name == name).and_then(|i| snapshot.outputs.get(i)).copied().unwrap_or(0.0)
        };
        for (base, ear, outward) in [(left_temple, value("EarLeft"), 1.0f32), (right_temple, value("EarRight"), -1.0)] {
            let side = Vec2::new(up.y, -up.x) * -outward;
            // Positive values stand the ear up, negative values fold it outward.
            let tilt = (-ear * 0.6).clamp(-0.6, 0.6) * outward;
            let (sin, cos) = tilt.sin_cos();
            let dir = Vec2::new(up.x * cos - up.y * sin, up.x * sin + up.y * cos);
            let base = base + up * size * 0.15;
            let tip = base + dir * size * (1.0 + ear.max(0.0) * 0.25);
            let a = base - side * size * 0.35;
            let b = base + side * size * 0.35;
            painter.add(Shape::convex_polygon(
                vec![a, tip, b],
                self.pal.accent.gamma_multiply(0.35),
                Stroke::new(1.5, self.pal.accent),
            ));
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        // Wraps onto a second line in narrow windows instead of cutting buttons off.
        ui.horizontal_wrapped(|ui| {
            let selected = self
                .cameras
                .iter()
                .find(|c| c.index == s.camera_index)
                .map(|c| format!("{}: {}", c.index, c.name))
                .unwrap_or_else(|| format!("{}: {}", s.camera_index, self.t(s, "camera")));
            egui::ComboBox::from_id_salt("camera")
                .width(190.0)
                .selected_text(format!("📷 {selected}"))
                .truncate()
                .show_ui(ui, |ui| {
                    for camera in &self.cameras {
                        ui.selectable_value(
                            &mut s.camera_index,
                            camera.index,
                            format!("{}: {}", camera.index, camera.name),
                        );
                    }
                });
            if ui.button("⟳").on_hover_text(self.t(s, "refresh_cameras")).clicked() {
                self.cameras = camera::list();
            }
            let hint = self.t(s, "mirror");
            self.toggle(ui, &mut s.mirror, "⇆").on_hover_text(hint);
            ui.add_space(6.0);
            let (preview, landmarks) = (self.t(s, "camera_preview"), self.t(s, "face_landmarks"));
            let ears = format!("{}\n{}", self.t(s, "cat_ears"), self.t(s, "cat_ears_hint"));
            self.toggle(ui, &mut s.show_camera, &format!("🎥 {preview}"));
            self.toggle(ui, &mut s.show_landmarks, &format!("☺ {landmarks}"));
            self.toggle(ui, &mut s.cat_ears, "🐱").on_hover_text(ears);
            ui.add_space(6.0);
            if ui
                .button(format!("☺ {}", self.t(s, "capture_neutral")))
                .on_hover_text(self.t(s, "neutral_hint"))
                .clicked()
            {
                let _ = self.commands.send(Command::CaptureNeutral);
            }
            if ui.button("↺").on_hover_text(self.t(s, "neutral_undo")).clicked() {
                let _ = self.commands.send(Command::UndoNeutral);
            }
            if ui
                .button(format!("◎ {}", self.t(s, "center_gaze")))
                .on_hover_text(self.t(s, "center_gaze_hint"))
                .clicked()
            {
                let _ = self.commands.send(Command::CenterGaze);
            }
            if ui.button("↺").on_hover_text(self.t(s, "reset_gaze")).clicked() {
                let _ = self.commands.send(Command::ResetGaze);
            }
            let hint = self.t(s, "search");
            let search = egui::TextEdit::singleline(&mut self.search).hint_text(hint).desired_width(200.0);
            if ui.available_width() >= 210.0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.add(search));
            } else {
                ui.add(search);
            }
        });
    }

    fn avatar_known(snapshot: &Snapshot) -> bool {
        !snapshot.avatar_ft_params.is_empty()
    }

    fn used_channels(&self, snapshot: &Snapshot) -> [bool; COUNT] {
        let known = Self::avatar_known(snapshot);
        let mut used = [!known; COUNT];
        if known {
            for (i, binding) in snapshot.bindings.iter().enumerate() {
                if binding.is_some() {
                    self.mapper.dependencies(i).iter().for_each(|&d| used[d] = true);
                }
            }
        }
        used
    }

    /// Order of the Unified Expressions list. When the hovered channel feeds expressions
    /// that are not all in view, they are taken out and put back as one block lined up
    /// with the channel's row. The decision is kept while the
    /// channel stays hovered, otherwise the moved rows would count as visible next frame.
    /// Returns the order and the positions of the block in it.
    fn pull_related(&mut self, shown: &[usize], content_top: f32) -> (Vec<usize>, std::ops::Range<usize>) {
        let Some(Hover::Channel(c)) = self.hover_now else {
            self.pulled = None;
            return (shown.to_vec(), 0..0);
        };
        let feeds = |o: &usize| self.mapper.dependencies(*o).contains(&c);
        let related: Vec<usize> = shown.iter().copied().filter(feeds).collect();
        let rest: Vec<usize> = shown.iter().copied().filter(|o| !feeds(o)).collect();
        let at = match self.pulled {
            Some((pulled, at)) if pulled == c => at,
            _ => {
                let view = self.prev_rows.view[1];
                let in_view = related.iter().all(|o| {
                    self.prev_rows
                        .outputs
                        .iter()
                        .any(|(i, r)| i == o && r.top() >= view.top() - 0.5 && r.bottom() <= view.bottom() + 0.5)
                });
                let y = self.rows.channels.iter().find(|(i, _)| *i == c).map(|(_, r)| r.center().y);
                let at = match y {
                    Some(y) if !in_view && !related.is_empty() => {
                        let n = related.len() as f32;
                        let index = (y - content_top) / ROW_HEIGHT - n / 2.0;
                        // Keep the whole block inside the visible part of the list.
                        let first = ((view.top() - content_top) / ROW_HEIGHT).ceil();
                        let last = ((view.bottom() - content_top) / ROW_HEIGHT).floor() - n;
                        Some(index.round().min(last).max(first).max(0.0) as usize)
                    }
                    _ => None,
                };
                self.pulled = Some((c, at));
                at
            }
        };
        match at {
            Some(at) => {
                let at = at.min(rest.len());
                let mut order = rest;
                order.splice(at..at, related.iter().copied());
                (order, at..at + related.len())
            }
            None => (shown.to_vec(), 0..0),
        }
    }

    fn channel_label(&self, s: &Settings, index: usize) -> String {
        if s.friendly_names { channels::label(index) } else { channels::name(index).to_string() }
    }

    #[allow(clippy::too_many_arguments)]
    fn channel_cell(
        &self,
        ui: &mut egui::Ui,
        s: &Settings,
        snapshot: &Snapshot,
        index: usize,
        used: bool,
        width: f32,
        avatar: &str,
        highlight: bool,
    ) -> egui::Response {
        let pal = self.pal;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
        let painter = ui.painter_at(rect);
        if response.hovered() || highlight {
            painter.rect_filled(rect, CornerRadius::ZERO, pal.line.gamma_multiply(0.45));
        }
        let color = if used { pal.text } else { pal.dim };
        // Room for the value on the right and the markers after the name.
        let galley =
            truncated(&painter, self.channel_label(s, index), FontId::monospace(12.5), color, rect.width() - 90.0);
        let text_pos = rect.left_center() + Vec2::new(10.0, -galley.size().y / 2.0);
        let text_right = text_pos.x + galley.size().x;
        painter.galley(text_pos, galley, color);
        let mut x = text_right + 7.0;
        painter.circle_filled(Pos2::new(x, rect.center().y), 2.2, if used { pal.accent } else { pal.line });
        let calibration = self.shared.calibrations.lock().avatars.get(avatar).map(|c| c.get(index)).unwrap_or_default();
        if !calibration.is_default() {
            x += 9.0;
            painter.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, "◆", FontId::monospace(8.0), pal.accent);
        }
        if s.processing.source(index) == Source::Mesh {
            x += 11.0;
            painter.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, "▦", FontId::monospace(10.0), pal.dim);
        }
        let value = snapshot.values[index];
        painter.text(
            rect.right_center() - Vec2::new(10.0, 0.0),
            Align2::RIGHT_CENTER,
            percent(value),
            FontId::monospace(11.5),
            pal.dim,
        );
        row_separator(&painter, rect, &pal);
        response.on_hover_text(format!(
            "{} · {} {}",
            channels::name(index),
            percent(snapshot.input[index]),
            self.t(s, "current_input")
        ))
    }

    /// Calibration, source, symmetry and smoothing for one row (one channel or a L/R pair).
    fn row_editor(&self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, members: &[usize], avatar: &str) {
        let pal = self.pal;
        let mut store = self.shared.calibrations.lock();
        let calibration = store.avatars.entry(avatar.to_string()).or_default();
        let mut changed = false;
        Frame::new().fill(pal.bg).stroke(Stroke::new(1.0, pal.line)).inner_margin(Margin::same(8)).show(ui, |ui| {
            // The channel list lays rows out without spacing; the editor needs the normal gaps.
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
            ui.horizontal_wrapped(|ui| {
                if members.len() > 1 {
                    let mut linked = calibration.linked(members[0]);
                    let text = if linked { "L = R" } else { "L ≠ R" };
                    let hint = if linked { self.t(s, "linked") } else { self.t(s, "unlinked") };
                    if self.toggle(ui, &mut linked, text).on_hover_text(hint).clicked() {
                        calibration.set_linked(members[0], linked);
                        changed = true;
                    }
                }
                if processing::has_mesh_source(members[0]) {
                    ui.label(RichText::new(self.t(s, "source")).color(pal.dim));
                    for (source, key) in [(Source::Model, "model"), (Source::Mesh, "mesh")] {
                        let mut on = s.processing.source(members[0]) == source;
                        if self.toggle(ui, &mut on, self.t(s, key)).clicked() {
                            for &m in members {
                                s.processing.sources.insert(channels::name(m).to_string(), source);
                            }
                        }
                    }
                }
                if let Some(group) = channels::symmetry_group(members[0]) {
                    let mut on = s.processing.group_symmetric(group);
                    if ui.checkbox(&mut on, self.t(s, "symmetry")).changed() {
                        s.processing.symmetry.insert(group.to_string(), on);
                    }
                }
                ui.label(RichText::new(self.t(s, "smoothing")).color(pal.dim));
                let mut strength = s.processing.smoothing_for(members[0]);
                if ui.add(egui::Slider::new(&mut strength, 0.0..=1.0).show_value(false)).changed() {
                    for &m in members {
                        s.processing.smoothing_channels.insert(channels::name(m).to_string(), strength);
                    }
                }
                ui.label(if strength > 0.0 { percent(strength) } else { self.t(s, "smoothing_off").to_string() });
            });
            ui.add_space(4.0);
            let sides: Vec<usize> =
                if members.len() > 1 && calibration.linked(members[0]) { vec![members[0]] } else { members.to_vec() };
            for &index in &sides {
                let mut c = calibration.get(index);
                ui.horizontal(|ui| {
                    if sides.len() > 1 {
                        ui.label(
                            RichText::new(if channels::name(index).ends_with("Left") { "L" } else { "R" }).strong(),
                        );
                    }
                    ui.label(RichText::new(self.t(s, "range")).color(pal.dim));
                    let mut min = c.min;
                    let mut max = c.max;
                    let a = ui.add(egui::Slider::new(&mut min, 0.0..=1.0).show_value(false));
                    let b = ui.add(egui::Slider::new(&mut max, 0.0..=1.0).show_value(false));
                    ui.label(format!("{}–{}", percent(min), percent(max)));
                    ui.label(RichText::new(format!("· {}", percent(snapshot.input[index]))).color(pal.dim))
                        .on_hover_text(self.t(s, "current_input"));
                    if a.changed() || b.changed() {
                        c.min = min.min(max - 0.05).max(0.0);
                        c.max = max.max(c.min + 0.05).min(1.0);
                        calibration.set(index, c);
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    let mut overriding = c.overriding().is_some();
                    if ui.checkbox(&mut overriding, self.t(s, "override")).changed() {
                        c.override_value = Some(c.override_value.unwrap_or(0.0));
                        c.override_disabled = !overriding;
                        calibration.set(index, c);
                        changed = true;
                    }
                    if overriding {
                        let mut value = c.override_value.unwrap_or(0.0);
                        if ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false)).changed() {
                            c.override_value = Some(value);
                            calibration.set(index, c);
                            changed = true;
                        }
                        ui.label(percent(value));
                    }
                });
            }
            if ui.button(self.t(s, "reset_rows")).clicked() {
                for &m in members {
                    calibration.set(m, ChannelCalibration::default());
                    let name = channels::name(m);
                    s.processing.smoothing_channels.remove(name);
                    s.processing.sources.remove(name);
                }
                if let Some(group) = channels::symmetry_group(members[0]) {
                    s.processing.symmetry.remove(group);
                }
                changed = true;
            }
        });
        if changed {
            self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
        }
    }

    fn mediapipe_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let used = self.used_channels(snapshot);
        let tongue = s.tongue;
        let visible = move |i: usize| !channels::is_tongue(i) || tongue;
        let total = (1..COUNT).filter(|&i| visible(i)).count();
        let active = (1..COUNT).filter(|&i| visible(i) && used[i]).count();
        let avatar = snapshot.avatar_id.clone().unwrap_or_default();
        let related = self.related();
        let frame = self.panel_frame().show(ui, |ui| {
            let (dim, title) = (self.pal.dim, self.t(s, "mediapipe"));
            let title = |ui: &mut egui::Ui| {
                ui.label(RichText::new(format!("{active}/{}", total - active)).color(dim));
                ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
            };
            let controls = |ui: &mut egui::Ui| {
                self.switch(ui, &mut s.show_unused);
                ui.label(self.t(s, "unused"));
                self.switch(ui, &mut s.tongue);
                ui.label(self.t(s, "tongue"));
                if ui.button(self.t(s, "reset")).on_hover_text(self.t(s, "reset_rows")).clicked() {
                    s.processing = processing::ProcessingSettings::default();
                    if let Some(c) = self.shared.calibrations.lock().avatars.get_mut(&avatar) {
                        c.channels.clear();
                        c.unlinked.clear();
                    }
                    self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
                }
                let hint = self.t(s, "symmetric_hint");
                let label = self.t(s, "symmetric_face");
                self.toggle(ui, &mut s.processing.symmetric_face, label).on_hover_text(hint);
                let (name, off) = (self.t(s, "smoothing"), self.t(s, "smoothing_off"));
                let hint = self.t(s, "smoothing_hint");
                self.slider_box(ui, &mut s.processing.smoothing, name, off, 150.0).on_hover_text(hint);
            };
            header(ui, "mediapipe", title, controls);
            if s.tongue {
                match &snapshot.tongue {
                    TongueStatus::Missing => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(self.t(s, "tongue_missing")).color(self.pal.dim).small());
                            if ui.small_button(self.t(s, "import_tongue")).clicked() {
                                self.import_tongue();
                            }
                        });
                    }
                    TongueStatus::Error(e) => {
                        let text = format!("{}: {e}", self.t(s, "tongue_model"));
                        ui.label(RichText::new(text).color(self.pal.dim).small());
                    }
                    _ => {}
                }
            }
            ui.separator();
            egui::ScrollArea::vertical().id_salt("mp").auto_shrink([false, false]).show(ui, |ui| {
                self.rows.view[0] = ui.clip_rect();
                // Rows touch, like table lines; each group gets a dashed box.
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let full = ui.available_width();
                let half = (full / 2.0).floor();
                // Too narrow for left and right side by side: one channel per line.
                let single = half < 160.0;
                for group in channels::display_rows() {
                    let rows: Vec<_> = group
                        .into_iter()
                        .filter(|(l, r)| {
                            let show = |i: usize| {
                                visible(i)
                                    && (s.show_unused || used[i])
                                    && (self.matches(&channels::label(i)) || self.matches(channels::name(i)))
                            };
                            show(*l) || r.is_some_and(show)
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    let group_top = ui.cursor().top();
                    for (l, r) in rows {
                        let lines: Vec<Vec<(usize, f32)>> = match r {
                            Some(r) if single => vec![vec![(l, full)], vec![(r, full)]],
                            Some(r) => vec![vec![(l, half), (r, full - half)]],
                            None => vec![vec![(l, full)]],
                        };
                        let mut clicked = false;
                        for cells in lines {
                            clicked |= ui
                                .horizontal(|ui| {
                                    let mut clicked = false;
                                    for (i, width) in cells {
                                        let highlight = related.channels[i];
                                        let cell =
                                            self.channel_cell(ui, s, snapshot, i, used[i], width, &avatar, highlight);
                                        self.rows.channels.push((i, cell.rect));
                                        if cell.hovered() {
                                            self.hover_now = Some(Hover::Channel(i));
                                        }
                                        clicked |= cell.clicked();
                                        if width == half {
                                            let divider = Stroke::new(1.0, self.pal.line.gamma_multiply(0.6));
                                            ui.painter().vline(cell.rect.right(), cell.rect.y_range(), divider);
                                        }
                                    }
                                    clicked
                                })
                                .inner;
                        }
                        if clicked {
                            self.expanded = if self.expanded == Some(l) { None } else { Some(l) };
                        }
                        if self.expanded == Some(l) {
                            let members: Vec<usize> = std::iter::once(l).chain(r).collect();
                            self.row_editor(ui, s, snapshot, &members, &avatar);
                        }
                    }
                    let group = Rect::from_min_max(
                        Pos2::new(ui.max_rect().left(), group_top),
                        Pos2::new(ui.max_rect().left() + full, ui.cursor().top()),
                    );
                    dashed_rect(ui.painter(), group, self.pal.line);
                    ui.add_space(10.0);
                }
            });
        });
        self.rows.frame[0] = frame.response.rect;
        dashed_rect(ui.painter(), frame.response.rect, self.pal.line);
    }

    fn unified_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let visible: Vec<usize> = (0..OUTPUTS.len()).filter(|&i| !OUTPUTS[i].cat_ears || s.cat_ears).collect();
        let bound = visible.iter().filter(|&&i| snapshot.bindings.get(i).is_some_and(Option::is_some)).count();
        let known = Self::avatar_known(snapshot);
        let related = self.related();
        let frame = self.panel_frame().show(ui, |ui| {
            let active = if known { bound } else { 0 };
            let count = format!("{active}/{}", visible.len() - active);
            let (title, hint) = (format!("{} ↗", self.t(s, "unified")), self.t(s, "mapping_docs"));
            let title = |ui: &mut egui::Ui| {
                ui.label(RichText::new(count).color(pal.dim));
                let link =
                    egui::Label::new(RichText::new(title).strong().color(pal.text)).truncate().sense(Sense::click());
                let link = ui.add(link).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hint);
                if link.clicked() {
                    crate::system::open_url(MAPPING_DOCS);
                }
            };
            let controls = |ui: &mut egui::Ui| {
                self.switch(ui, &mut s.show_unused);
                ui.label(self.t(s, "unused"));
            };
            header(ui, "unified", title, controls);
            ui.separator();
            egui::ScrollArea::vertical().id_salt("ue").auto_shrink([false, false]).show(ui, |ui| {
                self.rows.view[1] = ui.clip_rect();
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let shown: Vec<usize> = visible
                    .iter()
                    .copied()
                    .filter(|&i| {
                        let output = &OUTPUTS[i];
                        let is_bound = known && snapshot.bindings.get(i).is_some_and(Option::is_some);
                        (s.show_unused || !known || is_bound)
                            && (self.matches(output.name) || self.matches(output.friendly))
                    })
                    .collect();
                let content_top = ui.max_rect().top();
                let (order, block) = self.pull_related(&shown, content_top);
                let mut block_rect = Rect::NOTHING;
                for (n, &i) in order.iter().enumerate() {
                    let output = &OUTPUTS[i];
                    let is_bound = known && snapshot.bindings.get(i).is_some_and(Option::is_some);
                    let value = snapshot.outputs.get(i).copied().unwrap_or_default();
                    let text = output.label(s.friendly_names);
                    let row = value_row(ui, &pal, text, value, is_bound || !known, None, related.outputs[i]);
                    if block.contains(&n) {
                        block_rect = block_rect.union(row.rect);
                    }
                    self.rows.outputs.push((i, row.rect));
                    if row.hovered() {
                        self.hover_now = Some(Hover::Output(i));
                    }
                    row.on_hover_text(output.name);
                }
                if block_rect.is_positive() {
                    ui.painter().rect_stroke(
                        block_rect,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, pal.dim),
                        StrokeKind::Inside,
                    );
                }
            });
        });
        self.rows.frame[1] = frame.response.rect;
        dashed_rect(ui.painter(), frame.response.rect, self.pal.line);
    }

    fn avatar_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let known = Self::avatar_known(snapshot);
        let bound: Vec<(usize, &Binding)> =
            snapshot.bindings.iter().enumerate().filter_map(|(i, b)| b.as_ref().map(|b| (i, b))).collect();
        let matched: BTreeSet<&str> = bound.iter().map(|(i, _)| OUTPUTS[*i].name).collect();
        let unmatched: Vec<&String> = snapshot
            .avatar_ft_params
            .iter()
            .filter(|p| {
                let base = p.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches("Negative");
                !matched.contains(base) && !matched.contains(p.as_str())
            })
            .collect();
        let related = self.related();
        let frame = self.panel_frame().show(ui, |ui| {
            let count = format!("{}/{}", if known { bound.len() } else { 0 }, unmatched.len());
            let name =
                snapshot.avatar_name.clone().or_else(|| snapshot.avatar_id.clone()).unwrap_or_else(|| "—".into());
            let title = format!("{} {name}", self.t(s, "avatar"));
            let title = |ui: &mut egui::Ui| {
                ui.label(RichText::new(count).color(pal.dim));
                ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
            };
            let controls = |ui: &mut egui::Ui| {
                self.switch(ui, &mut s.show_unused);
                ui.label(self.t(s, "unused"));
            };
            header(ui, "avatar", title, controls);
            ui.separator();
            egui::ScrollArea::vertical().id_salt("avatar").auto_shrink([false, false]).show(ui, |ui| {
                self.rows.view[2] = ui.clip_rect();
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                if !known {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(self.t(s, "no_params")).color(pal.dim));
                        if !snapshot.vrchat_found {
                            ui.label(RichText::new(self.t(s, "no_vrchat")).color(pal.dim).small());
                        }
                        if !bound.is_empty() {
                            ui.label(RichText::new(self.t(s, "sending_default")).color(pal.dim).small());
                        }
                    });
                    return;
                }
                if bound.is_empty() {
                    ui.vertical_centered(|ui| ui.label(RichText::new(self.t(s, "no_ft_params")).color(pal.dim)));
                }
                for (i, binding) in &bound {
                    let output = &OUTPUTS[*i];
                    if !(self.matches(output.name) || self.matches(output.friendly)) {
                        continue;
                    }
                    let value = snapshot.outputs.get(*i).copied().unwrap_or_default();
                    let bits = (binding.bit_count() > 0).then(|| binding.encode(value));
                    let expanded = self.expanded_bindings.contains(i);
                    let arrow = if bits.is_none() {
                        ""
                    } else if expanded {
                        "▾ "
                    } else {
                        "▸ "
                    };
                    let label = format!("{arrow}{}", output.label(s.friendly_names));
                    let bit_boxes = bits.as_ref().map(|(b, _)| b.as_slice());
                    let response = value_row(ui, &pal, &label, value, true, bit_boxes, related.outputs[*i]);
                    self.rows.avatar.push((*i, response.rect));
                    if response.hovered() {
                        self.hover_now = Some(Hover::Avatar(*i));
                    }
                    if bits.is_some() && response.clicked() && !self.expanded_bindings.remove(i) {
                        self.expanded_bindings.insert(*i);
                    }
                    if expanded {
                        let (set, sign) = binding.encode(value);
                        let bits = binding.bits.iter().zip(set).chain(binding.negative.iter().map(|a| (a, sign)));
                        for (address, on) in bits {
                            bit_row(ui, &pal, address, on);
                        }
                    }
                }
                if s.show_unused {
                    for name in &unmatched {
                        if self.matches(name) {
                            value_row(ui, &pal, name, 0.0, false, None, false);
                        }
                    }
                }
            });
        });
        self.rows.frame[2] = frame.response.rect;
        dashed_rect(ui.painter(), frame.response.rect, self.pal.line);
    }

    /// Lines from the hovered row to the rows it feeds or is fed by (see `pull_related` for
    /// how a channel's expressions are brought into view).
    fn draw_links(&self, ui: &egui::Ui) {
        let Some(hover) = self.hover_now else { return };
        let (pal, rows) = (self.pal, &self.rows);
        let [mp, ue, avatar] = rows.frame;
        let painter = ui.painter();
        let stroke = Stroke::new(1.0, pal.text.gamma_multiply(0.85));
        let link = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
        let to_avatar = |output: usize, y: f32| {
            // In the narrow layout the avatar panel sits under the expressions; a line
            // would cut across them.
            if self.narrow {
                return;
            }
            if let Some(ay) = Rows::y(&rows.avatar, output, rows.view[2]) {
                link(Pos2::new(ue.right(), y), Pos2::new(avatar.left(), ay));
            }
        };
        match hover {
            Hover::Channel(c) => {
                let Some(y) = Rows::y(&rows.channels, c, rows.view[0]) else { return };
                for (o, _) in rows.outputs.iter().filter(|(o, _)| self.mapper.dependencies(*o).contains(&c)) {
                    if let Some(oy) = Rows::y(&rows.outputs, *o, rows.view[1]) {
                        link(Pos2::new(mp.right(), y), Pos2::new(ue.left(), oy));
                        to_avatar(*o, oy);
                    }
                }
            }
            Hover::Output(o) | Hover::Avatar(o) => {
                let Some(y) = Rows::y(&rows.outputs, o, rows.view[1]) else { return };
                for &c in self.mapper.dependencies(o) {
                    if let Some(cy) = Rows::y(&rows.channels, c, rows.view[0]) {
                        link(Pos2::new(mp.right(), cy), Pos2::new(ue.left(), y));
                    }
                }
                to_avatar(o, y);
            }
        }
    }

    fn import_tongue(&self) {
        let Some(path) = rfd::FileDialog::new().add_filter("ONNX model", &["onnx"]).pick_file() else { return };
        let dir = config::user_model_dir();
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&path, dir.join(tracker::TONGUE_FILE))) {
            Ok(_) => {
                let _ = self.commands.send(Command::ReloadModels);
            }
            Err(e) => log::warn!("importing tongue model: {e}"),
        }
    }

    fn device_name(&self, s: &Settings, device: Device) -> String {
        match device {
            Device::Auto => self.t(s, "auto").to_string(),
            Device::Cpu => self.t(s, "cpu").to_string(),
            Device::Gpu(i) => self
                .shared
                .gpus
                .iter()
                .find(|g| g.index == i)
                .map(|g| format!("GPU {i}: {}", g.name))
                .unwrap_or_else(|| format!("GPU {i}")),
        }
    }

    /// Device choice. `plain` draws it as text with a ▾, for use inside a stage box.
    fn device_picker(&self, ui: &mut egui::Ui, s: &Settings, id: &str, device: &mut Device, prefix: &str, plain: bool) {
        let width = ui.available_width() - 4.0;
        ui.scope(|ui| {
            if plain {
                let w = &mut ui.visuals_mut().widgets;
                for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
                    state.bg_stroke = Stroke::NONE;
                    state.weak_bg_fill = Color32::TRANSPARENT;
                }
                w.inactive.fg_stroke.color = self.pal.dim;
            }
            egui::ComboBox::from_id_salt(id)
                .width(width.max(80.0))
                .selected_text(format!("{prefix} {}", self.device_name(s, *device)).trim_start().to_string())
                .truncate()
                .show_ui(ui, |ui| {
                    let mut options = vec![Device::Auto, Device::Cpu];
                    options.extend(self.shared.gpus.iter().map(|g| Device::Gpu(g.index)));
                    for option in options {
                        ui.selectable_value(device, option, self.device_name(s, option));
                    }
                });
        });
    }

    /// One processing stage: title (with an optional help mark) and time on the first
    /// line, then `body` (the device) below, in a roomy outlined box.
    fn stage_box(
        &self,
        ui: &mut egui::Ui,
        title: &str,
        ms: Option<f32>,
        hint: Option<&str>,
        body: impl FnOnce(&mut egui::Ui),
    ) {
        let pal = self.pal;
        Frame::new().stroke(Stroke::new(1.0, pal.line)).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong());
                if let Some(hint) = hint {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    let color = if response.hovered() { pal.text } else { pal.dim };
                    ui.painter().circle_stroke(rect.center(), 7.0, Stroke::new(1.0, color));
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "?", FontId::monospace(10.0), color);
                    response.on_hover_text(hint);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(ms.map(|m| format!("{m:.2} ms")).unwrap_or_else(|| "—".into()));
                });
            });
            ui.add_space(4.0);
            body(ui);
        });
        ui.add_space(10.0);
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        egui::ScrollArea::vertical().id_salt("settings").show(ui, |ui| {
            egui::Grid::new("ports").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                ui.label(RichText::new(self.t(s, "vrchat_port")).color(pal.dim));
                ui.add(egui::DragValue::new(&mut s.send_port).range(1..=65535));
                ui.end_row();
                ui.label(RichText::new(self.t(s, "in_port")).color(pal.dim));
                ui.add(egui::DragValue::new(&mut s.receive_port).range(1..=65535));
                ui.end_row();
                ui.label(RichText::new(self.t(s, "osc_host")).color(pal.dim));
                ui.add(egui::TextEdit::singleline(&mut s.osc_host).desired_width(110.0));
                ui.end_row();
                ui.label(self.t(s, "interface_scale"));
                egui::ComboBox::from_id_salt("scale").selected_text(percent(s.interface_scale)).show_ui(ui, |ui| {
                    for scale in SCALES {
                        ui.selectable_value(&mut s.interface_scale, scale, percent(scale));
                    }
                });
                ui.end_row();
                ui.label(self.t(s, "idle_sleep")).on_hover_text(self.t(s, "idle_hint"));
                let current = if s.idle_sleep {
                    format!("{} {}", s.idle_minutes, self.t(s, "min"))
                } else {
                    self.t(s, "never").into()
                };
                egui::ComboBox::from_id_salt("idle").selected_text(current).show_ui(ui, |ui| {
                    if ui.selectable_label(!s.idle_sleep, self.t(s, "never")).clicked() {
                        s.idle_sleep = false;
                    }
                    for m in IDLE_MINUTES {
                        let label = format!("{m} {}", self.t(s, "min"));
                        if ui.selectable_label(s.idle_sleep && s.idle_minutes == m, label).clicked() {
                            s.idle_sleep = true;
                            s.idle_minutes = m;
                        }
                    }
                });
                ui.end_row();
                ui.label(self.t(s, "resolution"));
                let current = format!("{}×{}", s.resolution.0, s.resolution.1);
                egui::ComboBox::from_id_salt("resolution").selected_text(current).show_ui(ui, |ui| {
                    for r in RESOLUTIONS {
                        ui.selectable_value(&mut s.resolution, r, format!("{}×{}", r.0, r.1));
                    }
                });
                ui.end_row();
                ui.label(self.t(s, "camera_fps"));
                egui::ComboBox::from_id_salt("fps").selected_text(s.camera_fps.to_string()).show_ui(ui, |ui| {
                    for fps in [30, 60] {
                        ui.selectable_value(&mut s.camera_fps, fps, fps.to_string());
                    }
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            let (friendly, interp, interp_hint) =
                (self.t(s, "friendly_names"), self.t(s, "interpolation"), self.t(s, "interpolation_hint"));
            let (no_avatar, no_avatar_hint) = (self.t(s, "send_without_avatar"), self.t(s, "send_without_avatar_hint"));
            self.labeled_switch(ui, &mut s.friendly_names, friendly, None);
            self.labeled_switch(ui, &mut s.interpolation, interp, Some(interp_hint));
            self.labeled_switch(ui, &mut s.send_without_avatar, no_avatar, Some(no_avatar_hint));
            ui.separator();

            ui.label(RichText::new(self.t(s, "updates")).strong());
            self.updates_section(ui, s);
            ui.separator();

            let (debug, blur) = (self.t(s, "debug_performance"), self.t(s, "blur_preview"));
            self.labeled_switch(ui, &mut s.debug, debug, None);
            self.labeled_switch(ui, &mut s.camera_blur, blur, None);
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label(self.t(s, "processing_devices"));
                let mut all = if s.face_device == s.tongue_device { s.face_device } else { Device::Auto };
                let before = all;
                self.device_picker(ui, s, "all-devices", &mut all, self.t(s, "all"), false);
                if all != before {
                    s.face_device = all;
                    s.tongue_device = all;
                }
            });
            ui.add_space(8.0);
            if cfg!(not(feature = "directml")) {
                ui.label(
                    RichText::new("GPU inference needs a build with the `directml` feature.").color(pal.dim).small(),
                );
            }
            let perf = self.shared.perf.lock().clone();
            let avg = |stage: Stage| perf.stages.get(&stage).and_then(|st| st.average(Duration::from_secs(1)));
            let (capture, capture_hint) = (self.t(s, "camera_capture"), self.t(s, "capture_hint"));
            self.stage_box(ui, capture, avg(Stage::Camera), Some(capture_hint), |ui| {
                ui.label(RichText::new("Media Foundation").color(pal.dim));
            });
            self.stage_box(ui, "MediaPipe", avg(Stage::MediaPipe), None, |ui| {
                let mut d = s.face_device;
                self.device_picker(ui, s, "face-device", &mut d, "", true);
                s.face_device = d;
                if !snapshot.face_device.is_empty() && snapshot.face_device != self.device_name(s, d) {
                    ui.label(RichText::new(&snapshot.face_device).color(pal.dim).small());
                }
            });
            let tongue_ms = avg(Stage::Tongue).filter(|_| snapshot.tongue == TongueStatus::Ready);
            let tongue = self.t(s, "tongue");
            self.stage_box(ui, tongue, tongue_ms, None, |ui| {
                match &snapshot.tongue {
                    TongueStatus::Ready => {
                        let mut d = s.tongue_device;
                        self.device_picker(ui, s, "tongue-device", &mut d, "", true);
                        s.tongue_device = d;
                    }
                    TongueStatus::Off => {
                        ui.label(RichText::new(self.t(s, "enable_tongue")).color(pal.dim));
                    }
                    TongueStatus::Missing => {
                        ui.label(RichText::new(self.t(s, "tongue_missing")).color(pal.dim));
                    }
                    TongueStatus::Error(e) => {
                        ui.label(RichText::new(e).color(pal.dim));
                    }
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(self.t(s, "import_tongue")).clicked() {
                        self.import_tongue();
                    }
                    if tracker::tongue_path(&self.model_dir).is_some() {
                        ui.label(RichText::new(format!("✔ {}", self.t(s, "tongue_ready"))).color(pal.dim));
                    }
                });
            });
            ui.separator();

            if self.data_size.1.elapsed() > Duration::from_secs(10) {
                self.data_size = (config::dir_size(&config::data_dir()), Instant::now());
            }
            let armed = self.erase_armed.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
            let text = if armed {
                self.t(s, "erase_confirm").to_string()
            } else {
                format!("{} ({})", self.t(s, "erase"), mib(self.data_size.0))
            };
            let button = egui::Button::new(text).min_size(Vec2::new(ui.available_width(), 28.0));
            if ui.add(button).on_hover_text(self.t(s, "erase_hint")).clicked() {
                if armed {
                    if let Err(e) = config::erase_all() {
                        log::warn!("erasing app data: {e:#}");
                    }
                    // Restart without saving so nothing is written back.
                    let _ = updater::restart();
                } else {
                    self.erase_armed = Some(Instant::now());
                }
            }
        });
    }

    fn updates_section(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        ui.horizontal(|ui| {
            ui.label(self.t(s, "channel"));
            for (channel, key) in [(Channel::Stable, "stable"), (Channel::Beta, "beta")] {
                let mut on = s.update_channel == channel;
                if self.toggle(ui, &mut on, self.t(s, key)).clicked() {
                    s.update_channel = channel;
                }
            }
        });
        let state = std::mem::take(&mut *self.updates.lock());
        let next = match state {
            UpdateState::Idle => {
                if ui.button(self.t(s, "check_updates")).clicked() {
                    self.spawn_check(ui.ctx().clone());
                    UpdateState::Checking
                } else {
                    UpdateState::Idle
                }
            }
            UpdateState::Checking => {
                ui.label(self.t(s, "checking"));
                UpdateState::Checking
            }
            UpdateState::Downloading => {
                ui.label(self.t(s, "downloading"));
                UpdateState::Downloading
            }
            UpdateState::Failed(e) => {
                ui.label(RichText::new(format!("{}: {e}", self.t(s, "update_failed"))).color(self.pal.dim).small());
                if ui.button(self.t(s, "check_updates")).clicked() {
                    self.spawn_check(ui.ctx().clone());
                    UpdateState::Checking
                } else {
                    UpdateState::Failed(e)
                }
            }
            UpdateState::Ready(releases) => {
                let install = self.release_picker(ui, s, &releases);
                if let Some(release) = install {
                    self.spawn_install(release);
                    UpdateState::Downloading
                } else {
                    UpdateState::Ready(releases)
                }
            }
        };
        // A background check may have finished while we drew; its result wins.
        let mut guard = self.updates.lock();
        if matches!(*guard, UpdateState::Idle) {
            *guard = next;
        }
    }

    /// Version list for the selected channel; returns a release to install.
    fn release_picker(&mut self, ui: &mut egui::Ui, s: &Settings, releases: &[Release]) -> Option<Release> {
        let candidates: Vec<&Release> =
            releases.iter().filter(|r| s.update_channel == Channel::Beta || !r.prerelease).collect();
        if candidates.is_empty() {
            ui.label(RichText::new(self.t(s, "no_releases")).color(self.pal.dim));
            return None;
        }
        let latest = updater::latest(releases, s.update_channel).map(|r| r.tag.clone());
        let selected = self
            .selected_release
            .clone()
            .filter(|t| candidates.iter().any(|r| &r.tag == t))
            .or(latest.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(self.t(s, "version"));
            let shown = if Some(&selected) == latest.as_ref() {
                format!("{} · {selected}", self.t(s, "latest"))
            } else {
                selected.clone()
            };
            egui::ComboBox::from_id_salt("release").selected_text(shown).show_ui(ui, |ui| {
                for r in &candidates {
                    if ui.selectable_label(r.tag == selected, &r.tag).clicked() {
                        self.selected_release = Some(r.tag.clone());
                    }
                }
            });
        });
        let release = candidates.iter().find(|r| r.tag == selected)?;
        let mut install = None;
        ui.horizontal(|ui| {
            if release.version() == env!("CARGO_PKG_VERSION") {
                ui.label(RichText::new(format!("✔ {}", self.t(s, "up_to_date"))).color(self.pal.dim));
            } else {
                let newer = updater::is_newer(release.version(), env!("CARGO_PKG_VERSION"));
                let key = if newer { "update_now" } else { "install_restart" };
                if ui.add_enabled(release.zip_url.is_some(), egui::Button::new(self.t(s, key))).clicked() {
                    install = Some((*release).clone());
                }
            }
            if ui.link(self.t(s, "releases")).clicked() {
                crate::system::open_url(&release.page);
            }
        });
        install
    }

    fn spawn_check(&self, ctx: egui::Context) {
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            *updates.lock() = match updater::check() {
                Ok(releases) => UpdateState::Ready(releases),
                Err(e) => UpdateState::Failed(format!("{e:#}")),
            };
            ctx.request_repaint();
        });
    }

    fn spawn_install(&self, release: Release) {
        let updates = self.updates.clone();
        let settings = self.shared.settings.read().clone();
        let store = self.shared.calibrations.lock().clone();
        std::thread::spawn(move || {
            let _ = settings.save();
            let _ = config::save_calibrations(&store);
            if let Err(e) = updater::install(&release) {
                *updates.lock() = UpdateState::Failed(format!("{e:#}"));
            }
        });
    }

    fn theme_window(&mut self, ctx: &egui::Context, s: &mut Settings) {
        let mut open = self.theme_open;
        let title = self.t(s, "color_scheme");
        egui::Window::new(title).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            let language = s.language;
            egui::ComboBox::from_id_salt("scheme")
                .width(200.0)
                .selected_text(tr(language, s.scheme.text_key()))
                .show_ui(ui, |ui| {
                    for scheme in Scheme::ALL {
                        ui.selectable_value(&mut s.scheme, scheme, tr(language, scheme.text_key()));
                    }
                });
            if s.scheme == Scheme::Custom {
                ui.add_space(6.0);
                let outline = Stroke::new(1.0, self.pal.dim);
                egui::Grid::new("custom-colors").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                    let c = &mut s.custom_colors;
                    for (key, color) in [
                        ("theme_background", &mut c.background),
                        ("theme_panel", &mut c.panel),
                        ("theme_text", &mut c.text),
                        ("theme_accent", &mut c.accent),
                    ] {
                        ui.label(tr(language, key));
                        let swatch = ui.color_edit_button_srgb(color);
                        // Dark colours vanish against the window without an outline.
                        ui.painter().rect_stroke(swatch.rect, CornerRadius::ZERO, outline, StrokeKind::Outside);
                        ui.end_row();
                    }
                });
            }
        });
        self.theme_open = open;
    }

    fn performance_window(&self, ctx: &egui::Context, s: &mut Settings) {
        let pal = self.pal;
        let perf = self.shared.perf.lock().clone();
        let mut open = s.debug;
        let title = self.t(s, "debug_performance");
        egui::Window::new(title).open(&mut open).default_width(420.0).show(ctx, |ui| {
            ui.label(RichText::new(self.t(s, "latency")).strong());
            let latency = perf.latency.average(Duration::from_secs(1)).unwrap_or(0.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.t(s, "total_latency")).color(pal.dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(format!("{latency:.1} ms")));
            });
            let points: Vec<(f32, f32)> = perf.latency.history().collect();
            graph(ui, &pal, &points, 12.0, None);
            ui.add_space(6.0);

            ui.label(RichText::new(self.t(s, "memory")).strong());
            let now = Instant::now();
            let memory: Vec<(f32, f32)> = perf
                .memory
                .iter()
                .map(|(t, b)| (now.duration_since(*t).as_secs_f32(), *b as f32 / (1024.0 * 1024.0)))
                .collect();
            let first = memory.first().map(|m| m.1).unwrap_or(0.0);
            let last = memory.last().map(|m| m.1).unwrap_or(0.0);
            ui.horizontal(|ui| {
                ui.label(format!("{last:.0} MiB"));
                ui.label(RichText::new(format!("{:+.1} MiB", last - first)).color(pal.dim));
            });
            graph(ui, &pal, &memory, 600.0, Some(0.0));
            ui.add_space(6.0);

            ui.label(RichText::new(self.t(s, "averages")).color(pal.dim));
            egui::Grid::new("stages").num_columns(3).spacing([24.0, 4.0]).show(ui, |ui| {
                for stage in Stage::ALL {
                    let key = match stage {
                        Stage::Camera => "stage_camera",
                        Stage::MediaPipe => "stage_mediapipe",
                        Stage::Tongue => "stage_tongue",
                        Stage::Processing => "stage_processing",
                        Stage::Osc => "stage_osc",
                        Stage::Interface => "stage_interface",
                    };
                    let series = perf.stages.get(&stage);
                    let f = |over: u64| {
                        series
                            .and_then(|x| x.average(Duration::from_secs(over)))
                            .map(|v| format!("{v:.2}"))
                            .unwrap_or_else(|| "—".into())
                    };
                    ui.label(self.t(s, key));
                    ui.label(f(1));
                    ui.label(f(12));
                    ui.end_row();
                }
            });
        });
        s.debug = open;
    }

    fn sleeping(&self, s: &Settings) -> bool {
        s.idle_sleep && self.last_activity.elapsed() > Duration::from_secs(s.idle_minutes as u64 * 60)
    }
}

/// Line chart of (age seconds, value) points, newest on the right.
fn graph(ui: &mut egui::Ui, pal: &Palette, points: &[(f32, f32)], span: f32, floor: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.line), StrokeKind::Inside);
    if points.len() < 2 {
        return;
    }
    let lo = floor.unwrap_or_else(|| points.iter().map(|p| p.1).fold(f32::MAX, f32::min));
    let hi = points.iter().map(|p| p.1).fold(f32::MIN, f32::max).max(lo + 1e-3);
    let line: Vec<Pos2> = points
        .iter()
        .map(|(age, v)| {
            Pos2::new(
                rect.right() - age / span * rect.width(),
                rect.bottom() - 4.0 - (v - lo) / (hi - lo) * (rect.height() - 8.0),
            )
        })
        .collect();
    painter.add(Shape::line(line, Stroke::new(1.2, pal.accent)));
}

/// Parameter row: name and value, as one line of a table. `bits` draws a binary
/// encoding as boxes.
fn value_row(
    ui: &mut egui::Ui,
    pal: &Palette,
    name: &str,
    value: f32,
    active: bool,
    bits: Option<&[bool]>,
    highlight: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    paint_value_row(&painter, rect, pal, name, value, active, bits, highlight || response.hovered());
    response
}

#[allow(clippy::too_many_arguments)]
fn paint_value_row(
    painter: &egui::Painter,
    rect: Rect,
    pal: &Palette,
    name: &str,
    value: f32,
    active: bool,
    bits: Option<&[bool]>,
    highlight: bool,
) {
    if highlight {
        painter.rect_filled(rect, CornerRadius::ZERO, pal.line.gamma_multiply(0.45));
    }
    let color = if active { pal.text } else { pal.dim };
    let reserved = 64.0 + bits.map_or(0.0, |b| b.len() as f32 * 11.0 + 4.0);
    let galley = truncated(painter, name.to_string(), FontId::monospace(12.5), color, rect.width() - reserved);
    painter.galley(rect.left_center() + Vec2::new(10.0, -galley.size().y / 2.0), galley, color);
    let mut right = rect.right() - 10.0;
    if let Some(bits) = bits {
        for &on in bits.iter().rev() {
            let b = Rect::from_center_size(Pos2::new(right - 4.0, rect.center().y), Vec2::splat(8.0));
            if on {
                painter.rect_filled(b, CornerRadius::ZERO, pal.text);
            } else {
                painter.rect_stroke(b, CornerRadius::ZERO, Stroke::new(1.0, pal.dim), StrokeKind::Inside);
            }
            right -= 11.0;
        }
        right -= 4.0;
    }
    painter.text(
        Pos2::new(right, rect.center().y),
        Align2::RIGHT_CENTER,
        percent(value),
        FontId::monospace(11.5),
        pal.dim,
    );
    row_separator(painter, rect, pal);
}

/// One synced bool of a binary parameter, shown under its parameter: the parameter
/// name without its path, and ON / OFF.
fn bit_row(ui: &mut egui::Ui, pal: &Palette, address: &str, on: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    let name = address.rsplit('/').next().unwrap_or(address).to_string();
    let galley = truncated(&painter, name, FontId::monospace(12.0), pal.dim, rect.width() - 80.0);
    painter.galley(rect.left_center() + Vec2::new(26.0, -galley.size().y / 2.0), galley, pal.dim);
    let (text, color) = if on { ("ON", pal.text) } else { ("OFF", pal.dim) };
    painter.text(
        rect.right_center() - Vec2::new(10.0, 0.0),
        Align2::RIGHT_CENTER,
        text,
        FontId::monospace(11.5),
        color,
    );
    row_separator(&painter, rect, pal);
}

/// Thin line under a table row.
fn row_separator(painter: &egui::Painter, rect: Rect, pal: &Palette) {
    let y = rect.bottom() - 0.5;
    painter.hline(rect.x_range(), y, Stroke::new(1.0, pal.line.gamma_multiply(0.6)));
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let draw_started = Instant::now();
        let ctx = ui.ctx().clone();
        if ctx.input(|i| !i.events.is_empty() || i.pointer.velocity() != Vec2::ZERO) {
            self.last_activity = Instant::now();
        }
        let mut s = self.shared.settings.read().clone();
        let before = s.clone();

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

        let snapshot = self.shared.snapshot.lock().clone();
        self.update_texture(&ctx, &snapshot);
        self.hover = self.hover_now.take();
        self.prev_rows = std::mem::take(&mut self.rows);

        let bar = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 10));
        egui::Panel::top("top").frame(bar).show(ui, |ui| self.top_bar(ui, &mut s));
        if self.settings_open {
            let frame = Frame::new()
                .fill(self.pal.panel)
                .stroke(Stroke::new(1.0, self.pal.line))
                .inner_margin(Margin::same(14));
            egui::Panel::right("settings")
                .exact_size(360.0)
                .resizable(false)
                .frame(frame)
                .show(ui, |ui| self.settings_panel(ui, &mut s, &snapshot));
        }
        let body = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 4));
        egui::CentralPanel::default().frame(body).show(ui, |ui| {
            self.preview(ui, &s, &snapshot);
            ui.add_space(10.0);
            self.toolbar(ui, &mut s);
            ui.add_space(10.0);

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
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                column(ui, Vec2::new(mp, height), &mut |ui| self.mediapipe_panel(ui, &mut s, &snapshot));
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
        });
        if self.hover_now != self.hover {
            // Highlights follow the previous frame's hover; draw once more to catch up.
            ctx.request_repaint();
        }
        self.theme_window(&ctx, &mut s);
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
        self.shared.perf.lock().record(Stage::Interface, draw_started.elapsed().as_secs_f32() * 1000.0);
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
