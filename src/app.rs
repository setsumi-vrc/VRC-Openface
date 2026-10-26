use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect,
    RichText, Sense, Stroke, TextStyle, TextureHandle, TextureOptions, Vec2,
};

use crate::camera::{self, CameraEntry};
use crate::channels::{self, COUNT};
use crate::config::Settings;
use crate::processing::Smoothing;
use crate::tracker::{Command, Shared, Snapshot, Status};
use crate::unified::PARAMS;

const BG: Color32 = Color32::from_rgb(0, 0, 0);
const FG: Color32 = Color32::from_rgb(235, 235, 235);
const DIM: Color32 = Color32::from_rgb(110, 110, 110);
const LINE: Color32 = Color32::from_rgb(55, 55, 55);
const ACTIVE: Color32 = Color32::from_rgb(120, 220, 140);
const RESOLUTIONS: [(u32, u32); 4] = [(640, 480), (848, 480), (1280, 720), (1920, 1080)];

pub struct App {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    cameras: Vec<CameraEntry>,
    texture: Option<TextureHandle>,
    texture_seq: u64,
    search: String,
    settings_open: bool,
    rows: Vec<Vec<(usize, Option<usize>)>>,
    dependencies: Vec<Vec<usize>>,
    saved: Settings,
    last_save: Instant,
}

fn style(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    for (text_style, size) in [
        (TextStyle::Body, 12.5),
        (TextStyle::Button, 12.5),
        (TextStyle::Monospace, 12.5),
        (TextStyle::Small, 10.5),
        (TextStyle::Heading, 17.0),
    ] {
        style.text_styles.insert(text_style, FontId::new(size, FontFamily::Monospace));
    }
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = Color32::from_rgb(10, 10, 10);
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(12, 12, 12);
    v.override_text_color = Some(FG);
    v.selection.bg_fill = FG;
    v.selection.stroke = Stroke::new(1.0, BG);
    v.window_stroke = Stroke::new(1.0, LINE);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.bg_fill = BG;
    v.widgets.inactive.weak_bg_fill = BG;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(28, 28, 28);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, DIM);
    style.spacing.button_padding = Vec2::new(8.0, 5.0);
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    ctx.set_global_style(style);
}

/// White-on-black when on, like a pressed key.
fn toggle(ui: &mut egui::Ui, on: &mut bool, text: &str) -> bool {
    let (fg, bg) = if *on { (BG, FG) } else { (FG, BG) };
    let clicked = ui.add(egui::Button::new(RichText::new(text).color(fg)).fill(bg)).clicked();
    if clicked {
        *on = !*on;
    }
    clicked
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

fn panel_frame() -> Frame {
    Frame::new().stroke(Stroke::new(1.0, LINE)).inner_margin(Margin::same(8))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, shared: Arc<Shared>, commands: Sender<Command>) -> Self {
        style(&cc.egui_ctx);
        let saved = shared.settings.read().clone();
        Self {
            shared,
            commands,
            cameras: camera::list(),
            texture: None,
            texture_seq: 0,
            search: String::new(),
            settings_open: false,
            rows: channels::display_rows(),
            dependencies: PARAMS.iter().map(|p| p.dependencies()).collect(),
            saved,
            last_save: Instant::now(),
        }
    }

    fn matches(&self, text: &str) -> bool {
        self.search.is_empty() || text.to_lowercase().contains(&self.search.to_lowercase())
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

    fn persist(&mut self, settings: &Settings) {
        if self.last_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        let a = serde_json::to_string(settings).ok();
        if a != serde_json::to_string(&self.saved).ok() {
            if let Err(e) = settings.save() {
                log::warn!("saving settings: {e:#}");
            }
            self.saved = settings.clone();
        }
        self.last_save = Instant::now();
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        ui.horizontal(|ui| {
            let power = if s.tracking_enabled { "● FT" } else { "○ FT" };
            toggle(ui, &mut s.tracking_enabled, power);
            ui.add_space(8.0);
            ui.label(RichText::new("VRC OPENFACE").heading().strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    self.settings_open = !self.settings_open;
                }
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(DIM));
            });
        });
    }

    fn preview(&self, ui: &mut egui::Ui, s: &Settings, snapshot: &Snapshot) {
        let height = (ui.available_height() * 0.42).clamp(160.0, 430.0);
        let (outer, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, CornerRadius::ZERO, BG);

        let (fw, fh) = snapshot.frame_size;
        let aspect = if fw > 0 && fh > 0 { fw as f32 / fh as f32 } else { 16.0 / 9.0 };
        let image_w = (outer.height() * aspect).min(outer.width());
        let image = Rect::from_center_size(outer.center(), Vec2::new(image_w, image_w / aspect));

        // Hatched side margins.
        let hatch = Stroke::new(1.0, Color32::from_rgb(28, 28, 28));
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

        if s.preview
            && let Some(texture) = &self.texture
        {
            painter.image(texture.id(), image, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        if s.landmarks {
            let dot = if s.preview { ACTIVE } else { FG };
            for p in &snapshot.landmarks {
                let pos = Pos2::new(image.left() + p[0] * image.width(), image.top() + p[1] * image.height());
                painter.circle_filled(pos, 0.9, dot);
            }
        }
        painter.rect_stroke(outer, CornerRadius::ZERO, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);

        let status = match &snapshot.status {
            Status::Running => None,
            Status::Stopped => Some("Tracking stopped".to_string()),
            Status::Starting => Some("Starting camera…".to_string()),
            Status::NoFace => Some("No face detected".to_string()),
            Status::CameraError(e) => Some(format!("Camera error: {e}")),
            Status::ModelError(e) => Some(format!("Model error: {e}")),
        };
        if let Some(status) = status {
            painter.text(
                outer.left_top() + Vec2::new(14.0, 12.0),
                Align2::LEFT_TOP,
                status,
                FontId::monospace(12.0),
                FG,
            );
        }
        if let Some(progress) = snapshot.capture_progress {
            let text = format!("Hold still… {:.0}%", progress * 100.0);
            painter.text(
                outer.center_top() + Vec2::new(0.0, 12.0),
                Align2::CENTER_TOP,
                text,
                FontId::monospace(13.0),
                FG,
            );
        }
        let fps = format!("Tracking {:.0} FPS  |  Camera {:.0} FPS", snapshot.tracking_fps, snapshot.camera_fps);
        painter.text(
            outer.left_bottom() + Vec2::new(14.0, -12.0),
            Align2::LEFT_BOTTOM,
            fps,
            FontId::monospace(11.5),
            DIM,
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        ui.horizontal(|ui| {
            let selected = self
                .cameras
                .iter()
                .find(|c| c.index == s.camera_index)
                .map(|c| format!("{}: {}", c.index, c.name))
                .unwrap_or_else(|| format!("{}: Camera", s.camera_index));
            egui::ComboBox::from_id_salt("camera")
                .width(190.0)
                .selected_text(RichText::new(format!("📷 {selected}")))
                .truncate()
                .show_ui(ui, |ui| {
                    for camera in &self.cameras {
                        ui.selectable_value(
                            &mut s.camera_index,
                            camera.index,
                            format!("{}: {}", camera.index, camera.name),
                        );
                    }
                    if self.cameras.is_empty() {
                        ui.label(RichText::new("No cameras found").color(DIM));
                    }
                });
            if ui.button("⟳").on_hover_text("Refresh camera list").clicked() {
                self.cameras = camera::list();
            }
            toggle(ui, &mut s.flip_camera, "⇆ Flip");
            ui.add_space(6.0);
            toggle(ui, &mut s.preview, "Camera Preview");
            toggle(ui, &mut s.landmarks, "Face Landmarks");
            ui.add_space(6.0);
            let neutral = if s.calibration.neutral.is_some() { "☺ Neutral set" } else { "☺ Capture neutral" };
            if ui.button(neutral).on_hover_text("Relax your face and look at the camera").clicked() {
                let _ = self.commands.send(Command::CaptureNeutral);
            }
            if ui.button("↺").on_hover_text("Reset neutral").clicked() {
                let _ = self.commands.send(Command::ResetNeutral);
            }
            let gaze = if s.calibration.gaze_center.is_some() { "◎ Gaze centered" } else { "◎ Center gaze" };
            if ui.button(gaze).on_hover_text("Look straight ahead").clicked() {
                let _ = self.commands.send(Command::CenterGaze);
            }
            if ui.button("↺").on_hover_text("Reset gaze").clicked() {
                let _ = self.commands.send(Command::ResetGaze);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search).hint_text("Search parameter…").desired_width(200.0),
                );
            });
        });
    }

    fn used_channels(&self, snapshot: &Snapshot, avatar_known: bool) -> [bool; COUNT] {
        let mut used = [!avatar_known; COUNT];
        if avatar_known {
            for (deps, binding) in self.dependencies.iter().zip(&snapshot.bindings) {
                if binding.is_some() {
                    deps.iter().for_each(|&d| used[d] = true);
                }
            }
        }
        used
    }

    fn channel_cell(
        &self,
        ui: &mut egui::Ui,
        s: &mut Settings,
        snapshot: &Snapshot,
        index: usize,
        used: bool,
        width: f32,
    ) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
        let painter = ui.painter_at(rect);
        if response.hovered() {
            painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(16, 16, 16));
        }
        let label = channels::label(index);
        let galley = painter.text(
            rect.left_center() + Vec2::new(4.0, 0.0),
            Align2::LEFT_CENTER,
            &label,
            FontId::monospace(12.0),
            if used { FG } else { DIM },
        );
        painter.circle_filled(Pos2::new(galley.right() + 7.0, rect.center().y), 2.2, if used { ACTIVE } else { LINE });
        let value = snapshot.values[index];
        painter.text(
            rect.right_center() - Vec2::new(6.0, 0.0),
            Align2::RIGHT_CENTER,
            percent(value),
            FontId::monospace(11.0),
            DIM,
        );
        // Thin level bar along the bottom edge.
        let bar = Rect::from_min_size(
            rect.left_bottom() - Vec2::new(0.0, 2.0),
            Vec2::new(rect.width() * value.clamp(0.0, 1.0), 1.5),
        );
        painter.rect_filled(bar, CornerRadius::ZERO, if used { FG } else { DIM });
        painter
            .line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, Color32::from_rgb(24, 24, 24)));

        let gain = &mut s.calibration.gains[index];
        response
            .on_hover_text(format!(
                "{} · raw {} · gain ×{:.2}\nRight-click to adjust",
                channels::name(index),
                percent(snapshot.raw[index]),
                gain
            ))
            .context_menu(|ui| {
                ui.label(channels::label(index));
                ui.add(egui::Slider::new(gain, 0.0..=3.0).text("gain"));
                if ui.button("Reset gain").clicked() {
                    *gain = 1.0;
                }
            });
    }

    fn mediapipe_panel(&self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let avatar_known = snapshot.avatar_id.is_some() || snapshot.avatar_param_count > 0;
        let used = self.used_channels(snapshot, avatar_known);
        let tongue = s.tongue;
        let visible = move |i: usize| !channels::is_tongue(i) || tongue;
        let total = (1..COUNT).filter(|&i| visible(i)).count();
        let active = (1..COUNT).filter(|&i| visible(i) && used[i]).count();
        panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{active}/{total}")).color(DIM));
                ui.label(RichText::new("MediaPipe").strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle(ui, &mut s.show_unused, "Unused");
                    toggle(ui, &mut s.tongue, "Tongue");
                    if ui.button("Reset").on_hover_text("Reset all channel gains").clicked() {
                        s.calibration.gains.iter_mut().for_each(|g| *g = 1.0);
                    }
                    toggle(ui, &mut s.symmetric, "Symmetric face");
                    egui::ComboBox::from_id_salt("smoothing")
                        .selected_text(format!("Smoothing  {}", s.smoothing.label()))
                        .show_ui(ui, |ui| {
                            for option in Smoothing::ALL {
                                ui.selectable_value(&mut s.smoothing, option, option.label());
                            }
                        });
                });
            });
            if s.tongue
                && let Some(e) = &snapshot.tongue_error
            {
                ui.label(RichText::new(format!("Tongue: {e}")).color(DIM).small());
            }
            ui.separator();
            egui::ScrollArea::vertical().id_salt("mp").auto_shrink([false, false]).show(ui, |ui| {
                let half = (ui.available_width() - 6.0) / 2.0;
                for group in &self.rows {
                    let rows: Vec<_> = group
                        .iter()
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
                        ui.horizontal(|ui| match r {
                            Some(r) => {
                                self.channel_cell(ui, s, snapshot, *l, used[*l], half);
                                self.channel_cell(ui, s, snapshot, *r, used[*r], half);
                            }
                            None => self.channel_cell(ui, s, snapshot, *l, used[*l], half * 2.0 + 6.0),
                        });
                    }
                    ui.add_space(8.0);
                }
            });
        });
    }

    fn unified_panel(&self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let bound = snapshot.bindings.iter().filter(|b| b.is_some()).count();
        panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}/{bound}", PARAMS.len())).color(DIM));
                ui.label(RichText::new("Unified Expressions").strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle(ui, &mut s.show_unused, "Unused");
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("ue").auto_shrink([false, false]).show(ui, |ui| {
                for (i, param) in PARAMS.iter().enumerate() {
                    let is_bound = snapshot.bindings.get(i).is_some_and(Option::is_some);
                    if (!s.show_unused && !is_bound) || !self.matches(param.name) {
                        continue;
                    }
                    let value = snapshot.unified.get(i).copied().unwrap_or_default();
                    value_row(ui, param.name, value, is_bound);
                }
            });
        });
    }

    fn avatar_panel(&self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let bound: Vec<_> =
            snapshot.bindings.iter().enumerate().filter_map(|(i, b)| b.as_ref().map(|b| (i, b))).collect();
        let avatar_known = snapshot.avatar_id.is_some() || snapshot.avatar_param_count > 0;
        panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                let count = if avatar_known { bound.len() } else { 0 };
                ui.label(RichText::new(format!("{count}/{}", snapshot.avatar_param_count)).color(DIM));
                let id = snapshot.avatar_id.as_deref().map(short_id).unwrap_or("—".into());
                ui.label(RichText::new(format!("Avatar: {id}")).strong());
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("avatar").auto_shrink([false, false]).show(ui, |ui| {
                if !avatar_known {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Parameters not found").color(DIM));
                        let hint = if snapshot.vrchat_found {
                            "Waiting for avatar parameters…"
                        } else {
                            "VRChat OSCQuery not detected. Enable OSC in VRChat's action menu."
                        };
                        ui.label(RichText::new(hint).color(DIM).small());
                        if bound.iter().len() > 0 {
                            ui.label(RichText::new("Sending FT/v2 floats for every parameter.").color(DIM).small());
                        }
                    });
                    return;
                }
                if bound.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("No face tracking parameters on this avatar").color(DIM))
                    });
                }
                for (i, binding) in bound {
                    let text = binding.describe();
                    if !self.matches(&text) {
                        continue;
                    }
                    value_row(ui, &text, snapshot.unified.get(i).copied().unwrap_or_default(), true);
                }
            });
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context, s: &mut Settings, snapshot: &Snapshot) {
        let mut open = self.settings_open;
        egui::Window::new("Settings").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("settings").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label("Resolution");
                egui::ComboBox::from_id_salt("resolution")
                    .selected_text(format!("{}×{}", s.resolution.0, s.resolution.1))
                    .show_ui(ui, |ui| {
                        for r in RESOLUTIONS {
                            ui.selectable_value(&mut s.resolution, r, format!("{}×{}", r.0, r.1));
                        }
                    });
                ui.end_row();
                ui.label("Camera FPS");
                egui::ComboBox::from_id_salt("fps").selected_text(s.camera_fps.to_string()).show_ui(ui, |ui| {
                    for fps in [30, 60] {
                        ui.selectable_value(&mut s.camera_fps, fps, fps.to_string());
                    }
                });
                ui.end_row();
                ui.label("OSC address");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.osc_host).desired_width(120.0));
                    ui.add(egui::DragValue::new(&mut s.osc_port).range(1..=65535));
                });
                ui.end_row();
                ui.label("Without avatar info");
                ui.checkbox(&mut s.send_without_avatar, "Send FT/v2 floats for everything");
                ui.end_row();
                ui.label("Inference");
                ui.horizontal(|ui| {
                    let mut gpu = s.gpu_device.is_some();
                    if ui.checkbox(&mut gpu, "DirectML GPU").changed() {
                        s.gpu_device = gpu.then_some(0);
                    }
                    if let Some(device) = &mut s.gpu_device {
                        ui.add(egui::DragValue::new(device).range(0..=16).prefix("adapter "));
                    }
                });
                ui.end_row();
            });
            if cfg!(not(feature = "directml")) {
                ui.label(RichText::new("GPU inference needs a build with `--features directml`.").color(DIM).small());
            }
            if let Some(e) = &snapshot.osc_error {
                ui.label(RichText::new(format!("OSC: {e}")).color(DIM).small());
            }
        });
        self.settings_open = open;
    }
}

fn short_id(id: &str) -> String {
    if id.len() > 18 { format!("{}…", &id[..18]) } else { id.to_string() }
}

fn value_row(ui: &mut egui::Ui, name: &str, value: f32, active: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let color = if active { FG } else { DIM };
    painter.text(rect.left_center() + Vec2::new(4.0, 0.0), Align2::LEFT_CENTER, name, FontId::monospace(12.0), color);
    painter.text(
        rect.right_center() - Vec2::new(6.0, 0.0),
        Align2::RIGHT_CENTER,
        percent(value),
        FontId::monospace(11.0),
        DIM,
    );
    let mid = rect.center().x;
    let bar = if value >= 0.0 {
        Rect::from_min_size(Pos2::new(rect.left(), rect.bottom() - 2.0), Vec2::new(rect.width() * value.min(1.0), 1.5))
    } else {
        let w = rect.width() * 0.5 * (-value).min(1.0);
        Rect::from_min_size(Pos2::new(mid - w, rect.bottom() - 2.0), Vec2::new(w, 1.5))
    };
    painter.rect_filled(bar, CornerRadius::ZERO, color);
    painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, Color32::from_rgb(24, 24, 24)));
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let snapshot = self.shared.snapshot.lock().clone();
        let ctx = ui.ctx().clone();
        self.update_texture(&ctx, &snapshot);
        let mut s = self.shared.settings.read().clone();
        let before = s.clone();

        egui::Panel::top("top").frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(16, 10))).show(ui, |ui| {
            self.top_bar(ui, &mut s);
        });
        egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(16, 4))).show(
            ui,
            |ui| {
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
                        self.avatar_panel(ui, &snapshot);
                    });
                });
            },
        );
        self.settings_window(&ctx, &mut s, &snapshot);

        // Calibration may have changed on the tracker thread while we drew; only write
        // back the fields the UI touched.
        if serde_json::to_string(&s).ok() != serde_json::to_string(&before).ok() {
            let mut shared = self.shared.settings.write();
            let calibration = shared.calibration.clone();
            *shared = s.clone();
            shared.calibration.neutral = calibration.neutral;
            shared.calibration.gaze_center = calibration.gaze_center;
        }
        let current = self.shared.settings.read().clone();
        self.persist(&current);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let settings = self.shared.settings.read().clone();
        if let Err(e) = settings.save() {
            log::warn!("saving settings: {e:#}");
        }
    }
}
