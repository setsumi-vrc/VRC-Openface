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
const ROW_HEIGHT: f32 = 26.0;

#[derive(Default)]
enum UpdateState {
    #[default]
    Idle,
    Checking,
    Ready(Vec<Release>),
    Downloading,
    Failed(String),
}

pub struct App {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    model_dir: PathBuf,
    cameras: Vec<CameraEntry>,
    texture: Option<TextureHandle>,
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
        }
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
        Frame::new().stroke(Stroke::new(1.0, self.pal.line)).fill(self.pal.panel).inner_margin(Margin::same(8))
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
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for language in Language::ALL.iter().rev() {
                    let mut on = s.language == *language;
                    if self.toggle(ui, &mut on, language.native_name()).clicked() {
                        s.language = *language;
                    }
                }
                ui.add_space(8.0);
                let mut theme = self.theme_open;
                if self.toggle(ui, &mut theme, "🎨").on_hover_text(self.t(s, "color_scheme")).clicked() {
                    self.theme_open = theme;
                }
                let mut open = self.settings_open;
                if self.toggle(ui, &mut open, "⚙").on_hover_text(self.t(s, "settings")).clicked() {
                    self.settings_open = open;
                }
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(self.pal.dim));
            });
        });
    }

    fn preview(&self, ui: &mut egui::Ui, s: &Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let height = (ui.available_height() * 0.42).clamp(160.0, 430.0);
        let (outer, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
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
        ui.horizontal(|ui| {
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
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(hint).desired_width(200.0));
            });
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
    ) -> egui::Response {
        let pal = self.pal;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
        let painter = ui.painter_at(rect);
        if response.hovered() {
            painter.rect_filled(rect, CornerRadius::ZERO, pal.line.gamma_multiply(0.35));
        }
        let color = if used { pal.text } else { pal.dim };
        let label = self.channel_label(s, index);
        let galley = painter.text(
            rect.left_center() + Vec2::new(4.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(12.0),
            color,
        );
        let mut x = galley.right() + 7.0;
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
            rect.right_center() - Vec2::new(6.0, 0.0),
            Align2::RIGHT_CENTER,
            percent(value),
            FontId::monospace(11.0),
            pal.dim,
        );
        let bar = Rect::from_min_size(
            rect.left_bottom() - Vec2::new(0.0, 2.0),
            Vec2::new(rect.width() * value.clamp(0.0, 1.0), 1.5),
        );
        painter.rect_filled(bar, CornerRadius::ZERO, color);
        painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, pal.line.gamma_multiply(0.5)));
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
        self.panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{active}/{}", total - active)).color(self.pal.dim));
                ui.label(RichText::new(self.t(s, "mediapipe")).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                    let strength = s.processing.smoothing;
                    let value = if strength > 0.0 { percent(strength) } else { self.t(s, "smoothing_off").to_string() };
                    ui.add_sized(
                        [110.0, 18.0],
                        egui::Slider::new(&mut s.processing.smoothing, 0.0..=1.0).show_value(false),
                    )
                    .on_hover_text(self.t(s, "smoothing_hint"));
                    ui.label(format!("{} {value}", self.t(s, "smoothing")));
                });
            });
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
                let half = (ui.available_width() - 6.0) / 2.0;
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
                    for (l, r) in rows {
                        let clicked = ui
                            .horizontal(|ui| match r {
                                Some(r) => {
                                    let a = self.channel_cell(ui, s, snapshot, l, used[l], half, &avatar);
                                    let b = self.channel_cell(ui, s, snapshot, r, used[r], half, &avatar);
                                    a.clicked() || b.clicked()
                                }
                                None => {
                                    self.channel_cell(ui, s, snapshot, l, used[l], half * 2.0 + 6.0, &avatar).clicked()
                                }
                            })
                            .inner;
                        if clicked {
                            self.expanded = if self.expanded == Some(l) { None } else { Some(l) };
                        }
                        if self.expanded == Some(l) {
                            let members: Vec<usize> = std::iter::once(l).chain(r).collect();
                            self.row_editor(ui, s, snapshot, &members, &avatar);
                        }
                    }
                    ui.add_space(8.0);
                }
            });
        });
    }

    fn unified_panel(&self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let visible: Vec<usize> = (0..OUTPUTS.len()).filter(|&i| !OUTPUTS[i].cat_ears || s.cat_ears).collect();
        let bound = visible.iter().filter(|&&i| snapshot.bindings.get(i).is_some_and(Option::is_some)).count();
        let known = Self::avatar_known(snapshot);
        self.panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                let active = if known { bound } else { 0 };
                ui.label(RichText::new(format!("{active}/{}", visible.len() - active)).color(pal.dim));
                let title = RichText::new(format!("{} ↗", self.t(s, "unified"))).strong().color(pal.text);
                if ui.link(title).on_hover_text(self.t(s, "mapping_docs")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(MAPPING_DOCS));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.switch(ui, &mut s.show_unused);
                    ui.label(self.t(s, "unused"));
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("ue").auto_shrink([false, false]).show(ui, |ui| {
                for &i in &visible {
                    let output = &OUTPUTS[i];
                    let is_bound = known && snapshot.bindings.get(i).is_some_and(Option::is_some);
                    if (!s.show_unused && known && !is_bound)
                        || !(self.matches(output.name) || self.matches(output.friendly))
                    {
                        continue;
                    }
                    let value = snapshot.outputs.get(i).copied().unwrap_or_default();
                    let text = output.label(s.friendly_names);
                    value_row(ui, &pal, text, value, is_bound || !known, None).on_hover_text(output.name);
                }
            });
        });
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
        self.panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                let count = if known { bound.len() } else { 0 };
                ui.label(RichText::new(format!("{count}/{}", unmatched.len())).color(pal.dim));
                let name =
                    snapshot.avatar_name.clone().or_else(|| snapshot.avatar_id.clone()).unwrap_or_else(|| "—".into());
                ui.label(RichText::new(format!("{} {name}", self.t(s, "avatar"))).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.switch(ui, &mut s.show_unused);
                    ui.label(self.t(s, "unused"));
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("avatar").auto_shrink([false, false]).show(ui, |ui| {
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
                    let response = value_row(ui, &pal, &label, value, true, bits.as_ref().map(|(b, _)| b.as_slice()));
                    if bits.is_some() && response.clicked() && !self.expanded_bindings.remove(i) {
                        self.expanded_bindings.insert(*i);
                    }
                    if expanded {
                        let (set, sign) = binding.encode(value);
                        let short = |a: &str| a.trim_start_matches("/avatar/parameters/").to_string();
                        for (address, on) in binding.bits.iter().zip(set) {
                            ui.label(RichText::new(format!("   {} = {on}", short(address))).small().color(pal.dim));
                        }
                        if let Some(address) = &binding.negative {
                            ui.label(RichText::new(format!("   {} = {sign}", short(address))).small().color(pal.dim));
                        }
                    }
                }
                if s.show_unused {
                    for name in &unmatched {
                        if self.matches(name) {
                            value_row(ui, &pal, name, 0.0, false, None);
                        }
                    }
                }
            });
        });
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

    fn device_picker(&self, ui: &mut egui::Ui, s: &Settings, id: &str, device: &mut Device) {
        let label = |d: Device| match d {
            Device::Auto => self.t(s, "auto").to_string(),
            Device::Cpu => self.t(s, "cpu").to_string(),
            Device::Gpu(i) => self
                .shared
                .gpus
                .iter()
                .find(|g| g.index == i)
                .map(|g| format!("GPU {i}: {}", g.name))
                .unwrap_or_else(|| format!("GPU {i}")),
        };
        egui::ComboBox::from_id_salt(id).width(200.0).selected_text(label(*device)).truncate().show_ui(ui, |ui| {
            let mut options = vec![Device::Auto, Device::Cpu];
            options.extend(self.shared.gpus.iter().map(|g| Device::Gpu(g.index)));
            for option in options {
                ui.selectable_value(device, option, label(option));
            }
        });
    }

    fn stage_box(&self, ui: &mut egui::Ui, title: &str, ms: Option<f32>, device: &str, hint: Option<&str>) {
        Frame::new().stroke(Stroke::new(1.0, self.pal.line)).inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let r = ui.label(RichText::new(title).strong());
                if let Some(hint) = hint {
                    r.on_hover_text(hint);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(ms.map(|m| format!("{m:.2} ms")).unwrap_or_else(|| "—".into()));
                });
            });
            ui.label(RichText::new(device).color(self.pal.dim));
        });
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
                self.device_picker(ui, s, "all-devices", &mut all);
                if all != before {
                    s.face_device = all;
                    s.tongue_device = all;
                }
            });
            if cfg!(not(feature = "directml")) {
                ui.label(
                    RichText::new("GPU inference needs a build with the `directml` feature.").color(pal.dim).small(),
                );
            }
            let perf = self.shared.perf.lock().clone();
            let avg = |stage: Stage| perf.stages.get(&stage).and_then(|st| st.average(Duration::from_secs(1)));
            let capture = self.t(s, "camera_capture");
            self.stage_box(ui, capture, avg(Stage::Camera), "Media Foundation", Some(self.t(s, "capture_hint")));
            ui.horizontal(|ui| {
                ui.label(RichText::new("MediaPipe").color(pal.dim));
                let mut d = s.face_device;
                self.device_picker(ui, s, "face-device", &mut d);
                s.face_device = d;
            });
            self.stage_box(ui, "MediaPipe", avg(Stage::MediaPipe), &snapshot.face_device, None);
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.t(s, "tongue")).color(pal.dim));
                let mut d = s.tongue_device;
                self.device_picker(ui, s, "tongue-device", &mut d);
                s.tongue_device = d;
            });
            let tongue_device = match &snapshot.tongue {
                TongueStatus::Ready => snapshot.tongue_device.clone(),
                TongueStatus::Off => self.t(s, "enable_tongue").to_string(),
                TongueStatus::Missing => self.t(s, "tongue_missing").to_string(),
                TongueStatus::Error(e) => e.clone(),
            };
            let tongue_ms = avg(Stage::Tongue).filter(|_| snapshot.tongue == TongueStatus::Ready);
            self.stage_box(ui, self.t(s, "tongue"), tongue_ms, &tongue_device, None);
            ui.horizontal(|ui| {
                if ui.button(self.t(s, "import_tongue")).clicked() {
                    self.import_tongue();
                }
                if tracker::tongue_path(&self.model_dir).is_some() {
                    ui.label(RichText::new(format!("✔ {}", self.t(s, "tongue_ready"))).color(pal.dim));
                }
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
                ui.ctx().open_url(egui::OpenUrl::new_tab(&release.page));
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
            ui.horizontal_wrapped(|ui| {
                for scheme in Scheme::ALL {
                    let mut on = s.scheme == scheme;
                    if self.toggle(ui, &mut on, self.t(s, scheme.text_key())).clicked() {
                        s.scheme = scheme;
                    }
                }
            });
            if s.scheme == Scheme::Custom {
                ui.add_space(6.0);
                let language = s.language;
                egui::Grid::new("custom-colors").num_columns(2).show(ui, |ui| {
                    let c = &mut s.custom_colors;
                    for (key, color) in [
                        ("theme_background", &mut c.background),
                        ("theme_panel", &mut c.panel),
                        ("theme_text", &mut c.text),
                        ("theme_accent", &mut c.accent),
                    ] {
                        ui.label(tr(language, key));
                        ui.color_edit_button_srgb(color);
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

/// Parameter row: name, value and a bar (centred for signed values). `bits` draws a
/// binary encoding as boxes.
fn value_row(
    ui: &mut egui::Ui,
    pal: &Palette,
    name: &str,
    value: f32,
    active: bool,
    bits: Option<&[bool]>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, pal.line.gamma_multiply(0.35));
    }
    let color = if active { pal.text } else { pal.dim };
    painter.text(rect.left_center() + Vec2::new(4.0, 0.0), Align2::LEFT_CENTER, name, FontId::monospace(12.0), color);
    let mut right = rect.right() - 6.0;
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
        FontId::monospace(11.0),
        pal.dim,
    );
    let mid = rect.center().x;
    let bar = if value >= 0.0 {
        Rect::from_min_size(Pos2::new(rect.left(), rect.bottom() - 2.0), Vec2::new(rect.width() * value.min(1.0), 1.5))
    } else {
        let w = rect.width() * 0.5 * (-value).min(1.0);
        Rect::from_min_size(Pos2::new(mid - w, rect.bottom() - 2.0), Vec2::new(w, 1.5))
    };
    painter.rect_filled(bar, CornerRadius::ZERO, color);
    painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, pal.line.gamma_multiply(0.5)));
    response
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

        let bar = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 10));
        egui::Panel::top("top").frame(bar).show(ui, |ui| self.top_bar(ui, &mut s));
        if self.settings_open {
            let frame = Frame::new()
                .fill(self.pal.panel)
                .stroke(Stroke::new(1.0, self.pal.line))
                .inner_margin(Margin::same(14));
            egui::Panel::right("settings")
                .exact_size(360.0)
                .frame(frame)
                .show(ui, |ui| self.settings_panel(ui, &mut s, &snapshot));
        }
        let body = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 4));
        egui::CentralPanel::default().frame(body).show(ui, |ui| {
            self.preview(ui, &s, &snapshot);
            ui.add_space(10.0);
            self.toolbar(ui, &mut s);
            ui.add_space(10.0);

            let height = ui.available_height() - 4.0;
            let width = ui.available_width();
            let gap = 16.0;
            let mp = (width - gap * 2.0) * 0.48;
            let side = (width - gap * 2.0 - mp) / 2.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(Vec2::new(mp, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(Vec2::new(mp, height));
                    self.mediapipe_panel(ui, &mut s, &snapshot);
                });
                ui.allocate_ui_with_layout(Vec2::new(side, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(Vec2::new(side, height));
                    self.unified_panel(ui, &mut s, &snapshot);
                });
                ui.allocate_ui_with_layout(Vec2::new(side, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(Vec2::new(side, height));
                    self.avatar_panel(ui, &mut s, &snapshot);
                });
            });
        });
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
